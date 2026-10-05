//! Transitive version resolution: which revision of every repository the
//! workspace checks out, and where.
//!
//! Every repository that declares a dependency — the root included — makes
//! one *request* for it. Requests land in *slots*: one checkout each, keyed by
//! the repository, its compatibility class (the semver major) and whether it
//! is source or an artefact. In each slot the highest request wins, unless an
//! override says otherwise. Only revisions that are selected make requests of
//! their own, so the configs of the checkouts the workspace ends up with are
//! exactly what decided the result.
//!
//! Resolution runs as its own pass, before anything is cloned or moved, and
//! reads configs from git objects rather than working trees: raising B to a
//! new revision changes the `.gitscale.toml` that decides D. The one exception
//! is a checkout already sitting at the selected commit, whose own
//! `.gitscale.toml` is read from disk — the same file, plus any edit not yet
//! committed.
//!
//! Nothing here reads git history. Two versions of one stream compare as
//! versions; anything else — a branch, a commit, tags of different streams —
//! is decided by position in the graph: the request from the repository that
//! dominates the other wins, and two that neither dominates are an error. So a
//! shallow CI checkout resolves exactly as a developer machine does. The one
//! use of history is a warning, computed only where history is on local disk:
//! a winner that turns out to be behind what a losing request asked for.
//!
//! Where the objects come from is [`Repos`]'s business — see
//! [`crate::stores`] — so the rules here can be tested without git.

use anyhow::{anyhow, bail, Result};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{
    parse_dependency_config, ArtefactUse, GitScaleConfig, RepoEntry, CONFIG_FILENAME,
};
use crate::resolve::SymlinkEntry;
use crate::version::{self, Class, Version};

// ---------------------------------------------------------------------------
// What resolution reads
// ---------------------------------------------------------------------------

/// A lookup that could not be answered from what is on this machine — offline,
/// a repository never fetched. Not a failure of the workspace: `status` shows
/// the slot as `unresolved` and suggests `--fetch`.
#[derive(Debug)]
pub struct Unavailable(pub String);

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

pub fn unavailable(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Unavailable(message.into()))
}

fn is_unavailable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Unavailable>().is_some()
}

/// A repository's branches and tags, each with the commit it names (a tag
/// peeled to its commit), and the remote's default branch.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Refs {
    pub branches: BTreeMap<String, String>,
    pub tags: BTreeMap<String, String>,
    pub default_branch: Option<String>,
}

/// Source or artefact: a built artefact and the source tree are different
/// files, so they never share a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Source,
    Artefact,
}

impl Kind {
    /// An entry that replaces its checkout with an image is an artefact; one
    /// that overlays an image is a source checkout like any other.
    pub fn of(entry: &RepoEntry) -> Kind {
        if entry.is_artefact() {
            Kind::Artefact
        } else {
            Kind::Source
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Source => "source",
            Kind::Artefact => "artefact",
        }
    }
}

/// The repositories resolution reads, by URL. Every method may fail with
/// [`Unavailable`] when the answer is not on this machine.
pub trait Repos {
    fn refs(&self, url: &str, kind: Kind) -> Result<Refs>;
    /// The `.gitscale.toml` of a source checkout at `commit`, `None` when
    /// that commit has none. `revision` is what names the commit, for a
    /// remote that serves refs but not bare commits.
    fn config_at(&self, url: &str, commit: &str, revision: &str) -> Result<Option<String>>;
    /// The `.gitscale.toml` an artefact's image for `commit` carries.
    fn artefact_config(&self, url: &str, commit: &str) -> Result<Option<String>>;
    /// The remote's default branch: what a checkout nobody gives a revision
    /// follows. Asked only when such a checkout does not exist yet, since one
    /// that does is read at its own HEAD.
    fn default_branch(&self, url: &str, kind: Kind) -> Result<Option<String>>;
    /// Whether `ancestor` is `descendant` or in its history — asked only for
    /// a warning, and [`Unavailable`] wherever history is not on local disk.
    fn is_ancestor(&self, url: &str, kind: Kind, ancestor: &str, descendant: &str) -> Result<bool>;
    /// Make the repositories ready, in parallel where that helps. Optional.
    fn prepare(&self, _wanted: &[(String, Kind)]) {}
    /// A revision was asked of `url` that its refs do not have: when they
    /// were read offline, a fetch may bring it. Optional.
    fn unknown(&self, _url: &str, _kind: Kind) {}
    /// The commit a branch of this workspace's own — not the remote's —
    /// points at: where a topic is developed. `None` where there are no
    /// local branches, as in CI.
    fn local_branch(&self, _url: &str, _branch: &str) -> Option<String> {
        None
    }
}

/// The checkouts already in the workspace.
pub trait Checkouts {
    /// The `.gitscale.toml` of the checkout at `directory`, when that
    /// checkout sits exactly at `commit`: `Some(None)` when it has none,
    /// `None` when the checkout is missing or elsewhere.
    fn config_if_at(&self, directory: &str, kind: Kind, commit: &str) -> Option<Option<String>>;
    /// HEAD of the git checkout at `directory`.
    fn head(&self, directory: &str) -> Option<String>;
}

// ---------------------------------------------------------------------------
// What resolution produces
// ---------------------------------------------------------------------------

/// Branch, tag or commit: what a revision names. Consumers decide for
/// themselves which of these they treat as pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevKind {
    Branch,
    Tag,
    Sha,
}

impl RevKind {
    pub fn label(self) -> &'static str {
        match self {
            RevKind::Branch => "branch",
            RevKind::Tag => "tag",
            RevKind::Sha => "sha",
        }
    }
}

/// Why a revision won its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The only request with a revision.
    Only,
    Semver,
    Calver,
    /// Neither is a version the other can be compared with: the request
    /// from the repository that dominates the other wins.
    Position,
    /// Every request names the same commit.
    Equal,
    Override,
    /// The slot is on the workspace's topic: its branch of that name beats
    /// every request.
    Topic,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::Only => "only",
            Reason::Semver => "semver",
            Reason::Calver => "calver",
            Reason::Position => "position",
            Reason::Equal => "equal",
            Reason::Override => "override",
            Reason::Topic => "topic",
        }
    }

    /// As `status --why` says it.
    pub fn describe(self) -> &'static str {
        match self {
            Reason::Only => "only request",
            Reason::Semver => "highest semver",
            Reason::Calver => "highest calendar version",
            Reason::Position => "asked for by a repository above the other",
            Reason::Equal => "same commit",
            Reason::Override => "override",
            Reason::Topic => "topic branch",
        }
    }
}

/// The revision a slot checks out.
#[derive(Debug, Clone)]
pub struct Chosen {
    /// The winning request's own text: `1.2.4` stays `1.2.4`.
    pub revision: String,
    pub commit: String,
    pub kind: RevKind,
    pub reason: Reason,
    /// `only`, `root` (the root's own request won), `raised` (a dependency
    /// asked for more than the root), `highest` (the root asked for
    /// nothing) or `override`.
    pub resolution: &'static str,
    /// Who asked for it: `root`, or the requesting checkout's directory.
    pub by: String,
}

/// One request, as `status --why` and the JSON output show it.
#[derive(Debug, Clone)]
pub struct RequestView {
    /// `root`, or the directory of the checkout whose config made it.
    pub from: String,
    /// `dir@revision` of each checkout between the root and the requester,
    /// the requester included.
    pub chain: Vec<String>,
    /// The directory the requester declared it at.
    pub directory: String,
    pub revision: String,
    pub revision_kind: Option<RevKind>,
    /// The commit the revision names, when it was looked up.
    pub commit: Option<String>,
    pub is_override: bool,
    pub selected: bool,
    /// Beaten by an override from a repository that dominates the requester,
    /// although it asked for more.
    pub overruled_by: Option<String>,
    /// Asked for a commit the selected one is behind. Known only where the
    /// history is on local disk; a warning, never a different result.
    pub ahead: bool,
    /// Names a revision the repository does not have — only ever seen
    /// overruled by an override, since otherwise it fails resolution.
    pub missing: bool,
}

impl RequestView {
    /// `root → imports/b@v2.1.0`, the path the request came by.
    pub fn path(&self) -> String {
        std::iter::once("root".to_string())
            .chain(self.chain.iter().cloned())
            .collect::<Vec<_>>()
            .join(" → ")
    }
}

/// What resolution would have chosen for a topic slot without the topic: the
/// pin a merge of the topic ships.
#[derive(Debug, Clone)]
pub struct Pin {
    pub revision: String,
    pub commit: String,
    /// `root`, or the directory of the checkout whose config asked for it.
    pub by: String,
}

/// A slot on the workspace's topic.
#[derive(Debug, Clone)]
pub struct SlotTopic {
    /// The slot's branch of the topic: the topic itself, or `<topic>@v<major>`
    /// for a checkout of an older major.
    pub branch: String,
    pub commit: String,
    /// The root's store has a branch of that name: developed here. Otherwise
    /// only the remote has one, and the checkout follows it.
    pub developed: bool,
    pub pin: Option<Pin>,
}

/// One checkout of the workspace.
#[derive(Debug, Clone)]
pub struct Slot {
    pub directory: String,
    pub url: String,
    /// How the root's entry uses the repository's artefact; `None` for a
    /// plain source checkout, and for every implicit one but an artefact.
    pub artefact: Option<ArtefactUse>,
    pub recursive: bool,
    pub kind: Kind,
    pub class: Class,
    /// The root entry's own revision; `None` for an implicit slot.
    pub declared: Option<String>,
    pub implicit: bool,
    /// `None` when nobody gives a revision: the checkout follows the branch
    /// it is on, as an entry without a revision always has.
    pub chosen: Option<Chosen>,
    /// Why this slot's revision could not be resolved, when it could not.
    pub unresolved: Option<String>,
    /// Why its own dependencies could not be read, when they could not.
    pub unread: Option<String>,
    pub requests: Vec<RequestView>,
    /// How many checkouts this repository has, one per class.
    pub majors: usize,
    /// The commit the checkout goes to: the winner's, the default branch's
    /// head for a slot nobody gives a revision, or the topic branch's.
    pub commit: Option<String>,
    /// The slot's branch of the workspace's topic, when it may have one: the
    /// root is on a topic, and nothing holds the slot at its pin.
    pub branch: Option<String>,
    /// Set when the slot is on the workspace's topic.
    pub topic: Option<SlotTopic>,
    /// The requester whose config pins the topic branch, keeping this slot
    /// at its pin although its remote has the branch.
    pub pinned_by: Option<String>,
}

impl Slot {
    /// The entry every command operates on: the declared one, at the
    /// revision resolution chose — the topic branch, on a topic.
    pub fn entry(&self) -> RepoEntry {
        RepoEntry {
            directory: self.directory.clone(),
            repo_url: self.url.clone(),
            revision: match (&self.topic, &self.chosen, &self.declared) {
                (Some(topic), _, _) => topic.branch.clone(),
                (None, Some(chosen), _) => chosen.revision.clone(),
                // Unresolved: what the root says is the best there is.
                (None, None, Some(declared)) if self.unresolved.is_some() => declared.clone(),
                _ => String::new(),
            },
            artefact: self.artefact,
            recursive: self.recursive,
            ..RepoEntry::default()
        }
    }

    /// What the slot is held at off the topic: the selected revision, or the
    /// topic's pin.
    pub fn pin(&self) -> Option<Pin> {
        match (&self.topic, &self.chosen) {
            (Some(topic), _) => topic.pin.clone(),
            (None, Some(chosen)) => Some(Pin {
                revision: chosen.revision.clone(),
                commit: chosen.commit.clone(),
                by: chosen.by.clone(),
            }),
            _ => None,
        }
    }

    /// The first repository that asked for this one, for `implicit via`.
    pub fn first_requester(&self) -> Option<&str> {
        self.requests.first().map(|r| r.from.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Resolution {
    /// The root's entries, sorted by directory as the config is read, then
    /// implicit slots by directory.
    pub slots: Vec<Slot>,
    /// The dedup links to plant inside checkouts.
    pub links: Vec<SymlinkEntry>,
}

impl Resolution {
    pub fn entries(&self) -> Vec<RepoEntry> {
        self.slots.iter().map(Slot::entry).collect()
    }

    /// The entries `names` selects: all of them when none are given, and an
    /// error naming any directory no slot has.
    pub fn select(&self, names: &[String]) -> Result<Vec<RepoEntry>> {
        crate::config::filter_entries(&self.entries(), names)
    }

    pub fn slot(&self, directory: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.directory == directory)
    }

    /// The slot `name` means: its own directory, or the path of a link a
    /// repository has to it — how that repository names the dependency.
    pub fn find(&self, name: &str) -> Option<&Slot> {
        let name = name.trim_end_matches('/');
        self.slot(name).or_else(|| {
            let link = self.links.iter().find(|l| l.link_path == Path::new(name))?;
            self.slot(&link.target_path.to_string_lossy())
        })
    }

    /// The links planted inside the checkout at `directory`, relative to it:
    /// gitscale's, not the checkout owner's work.
    pub fn planted_in(&self, directory: &str) -> Vec<String> {
        let entries = self.entries();
        self.links
            .iter()
            .filter_map(|link| {
                let owner = crate::resolve::owning_entry(&link.link_path, &entries)?;
                (owner.directory == directory).then(|| {
                    link.link_path
                        .strip_prefix(&owner.directory)
                        .unwrap_or(&link.link_path)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .collect()
    }

    /// The slots on the workspace's topic.
    pub fn topic_slots(&self) -> Vec<&Slot> {
        self.slots.iter().filter(|s| s.topic.is_some()).collect()
    }
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct SlotKey {
    /// Normalized.
    url: String,
    class: Class,
    kind: Kind,
}

/// Who made a request: the root, or the checkout of a slot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Node {
    Root,
    Slot(SlotKey),
}

/// A revision looked up in its repository.
#[derive(Debug, Clone)]
struct RevInfo {
    commit: String,
    kind: RevKind,
    version: Option<Version>,
    /// `None` for a branch, a commit or a tag that is not a version: it has
    /// no class of its own, and joins the checkout its repository's versions
    /// are in.
    class: Option<Class>,
}

#[derive(Debug, Clone)]
enum Info {
    /// No revision: no opinion.
    Empty,
    Known(RevInfo),
    Unavailable(String),
    /// A revision the repository does not have, with the error that says
    /// so. Overruled by an override from a repository above the requester;
    /// an error otherwise.
    Missing(String),
}

/// A revision looked up once per repository and revision.
#[derive(Debug, Clone)]
enum Classified {
    Known(RevInfo),
    Missing(String),
    Failed(String),
}

/// The error for a revision a repository does not have — told apart from
/// one that could not be looked up at all.
#[derive(Debug)]
struct UnknownRevision(String);

impl fmt::Display for UnknownRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnknownRevision {}

#[derive(Debug, Clone)]
struct Req {
    requester: Node,
    entry: RepoEntry,
    url: String,
    kind: Kind,
    info: Info,
    /// The topic's own request: no repository made it.
    topic: bool,
}

impl Req {
    fn known(&self) -> Option<&RevInfo> {
        match &self.info {
            Info::Known(info) => Some(info),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
enum Choice {
    Follow,
    Unresolved(String),
    Picked {
        winner: usize,
        reason: Reason,
        resolution: &'static str,
        /// Requests an override overruled although they asked for more, each
        /// with the override that did it.
        overruled: Vec<(usize, usize)>,
        /// Requests for a commit the winner is behind, where that is known.
        ahead: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
struct SlotState {
    requests: Vec<Req>,
    choice: Choice,
    directory: String,
    root_entry: Option<RepoEntry>,
    /// The commit its config is read at, when it has one to read.
    commit: Option<String>,
    /// What expanding it found missing, offline.
    unexpanded: Option<String>,
    /// On the topic: its branch, the commit, whether it is developed here,
    /// and the request that would have won without it.
    topic: Option<TopicPick>,
    /// The slot's branch of the topic, when it may have one.
    branch: Option<String>,
    /// The requester that keeps it at its pin, pinning the topic branch.
    pinned_by: Option<String>,
}

#[derive(Debug, Clone)]
struct TopicPick {
    developed: bool,
    pinned: Option<usize>,
}

impl SlotState {
    fn chosen_revision(&self) -> Option<&str> {
        match &self.choice {
            Choice::Picked { winner, .. } => Some(&self.requests[*winner].entry.revision),
            _ => None,
        }
    }
}

/// One round's answer.
#[derive(Debug, Clone, Default)]
struct Round {
    slots: BTreeMap<SlotKey, SlotState>,
    dominators: Dominators,
}

/// `dom[n]`: every node all paths from the root to `n` pass through.
#[derive(Debug, Clone, Default)]
struct Dominators {
    dom: HashMap<Node, BTreeSet<Node>>,
}

impl Dominators {
    /// Whether `p` dominates `q`: the root dominates everything, and every
    /// node dominates itself.
    fn dominates(&self, p: &Node, q: &Node) -> bool {
        p == &Node::Root || p == q || self.dom.get(q).is_some_and(|set| set.contains(p))
    }
}

/// Generous: a graph without cycles settles in as many rounds as it is deep.
const MAX_ROUNDS: usize = 64;

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

pub struct Engine<'a> {
    config: &'a GitScaleConfig,
    /// The root repository's own URL, normalized, when it has one.
    root_url: Option<String>,
    repos: &'a dyn Repos,
    checkouts: &'a dyn Checkouts,
    allow: Vec<String>,
    /// Normalized URLs the root declares, which need no allowlist.
    declared: BTreeSet<String>,
    classified: RefCell<HashMap<(String, String), Classified>>,
    configs: RefCell<HashMap<ConfigKey, Result<Option<GitScaleConfig>, String>>>,
    /// Every dependency edge seen at any revision considered, between
    /// normalized URLs, for the cycle check.
    edges: RefCell<BTreeMap<String, BTreeSet<String>>>,
    /// How each URL was last labelled in a chain, for messages.
    labels: RefCell<BTreeMap<String, String>>,
    /// The workspace's topic branch, when the root is on one.
    topic: Option<String>,
}

/// A config read at a commit, by repository, commit and checkout directory —
/// the directory because a checkout at that commit is read from disk.
type ConfigKey = (String, String, String);

/// The node standing for the root in the URL graph when it has no URL.
const ROOT: &str = "<root>";

impl<'a> Engine<'a> {
    pub fn new(
        config: &'a GitScaleConfig,
        root_url: Option<&str>,
        repos: &'a dyn Repos,
        checkouts: &'a dyn Checkouts,
    ) -> Self {
        let declared = config
            .repos
            .iter()
            .map(|e| crate::urls::normalize(&e.repo_url))
            .collect();
        Engine {
            config,
            root_url: root_url.map(crate::urls::normalize),
            repos,
            checkouts,
            allow: allowlist(config, root_url),
            declared,
            classified: RefCell::new(HashMap::new()),
            configs: RefCell::new(HashMap::new()),
            edges: RefCell::new(BTreeMap::new()),
            labels: RefCell::new(BTreeMap::new()),
            topic: None,
        }
    }

    /// Resolve for a workspace on the topic `branch`: see [`crate::topic`].
    pub fn with_topic(mut self, branch: Option<String>) -> Self {
        self.topic = branch;
        self
    }

    pub fn resolve(&self) -> Result<Resolution> {
        let root_node = self.root_node();
        let roots: Vec<Req> = self
            .config
            .repos
            .iter()
            .map(|entry| self.request(Node::Root, entry))
            .collect();
        for req in &roots {
            self.edge(&root_node, &req.url)?;
        }
        self.repos.prepare(
            &roots
                .iter()
                .map(|r| (r.entry.repo_url.clone(), r.kind))
                .collect::<Vec<_>>(),
        );

        let mut previous: Option<Round> = None;
        for _ in 0..MAX_ROUNDS {
            let mut requests = roots.clone();
            let mut unexpanded = BTreeMap::new();
            if let Some(round) = &previous {
                let (more, missing) = self.expand(round)?;
                requests.extend(more);
                unexpanded = missing;
            }
            let mut round = self.round(requests, previous.as_ref())?;
            for (key, message) in unexpanded {
                if let Some(state) = round.slots.get_mut(&key) {
                    state.unexpanded = Some(message);
                }
            }
            if previous
                .as_ref()
                .is_some_and(|p| fingerprint(p) == fingerprint(&round))
            {
                return self.finish(round);
            }
            previous = Some(round);
        }
        bail!(
            "dependency resolution did not settle after {} rounds",
            MAX_ROUNDS
        )
    }

    fn root_node(&self) -> String {
        self.root_url.clone().unwrap_or_else(|| ROOT.to_string())
    }

    fn request(&self, requester: Node, entry: &RepoEntry) -> Req {
        Req {
            requester,
            url: crate::urls::normalize(&entry.repo_url),
            kind: Kind::of(entry),
            entry: entry.clone(),
            info: Info::Empty,
            topic: false,
        }
    }

    /// Record a dependency edge, failing on the first cycle it closes.
    fn edge(&self, from: &str, to: &str) -> Result<()> {
        if from == to {
            bail!(
                "cycle: {} depends on itself",
                self.labels
                    .borrow()
                    .get(from)
                    .cloned()
                    .unwrap_or_else(|| from.to_string())
            );
        }
        self.edges
            .borrow_mut()
            .entry(from.to_string())
            .or_default()
            .insert(to.to_string());
        if let Some(path) = self.path_between(to, from) {
            let labels = self.labels.borrow();
            let root = self.root_node();
            let name = |url: &String| {
                if *url == root {
                    "root".to_string()
                } else {
                    labels.get(url).cloned().unwrap_or_else(|| url.clone())
                }
            };
            let mut chain: Vec<String> = std::iter::once(from.to_string())
                .chain(path)
                .map(|u| name(&u))
                .collect();
            chain.dedup();
            bail!(
                "cycle: {}. Remove the dependency from the repository that closes the loop",
                chain.join(" → ")
            );
        }
        Ok(())
    }

    /// A path of edges from `from` to `to`, both ends included.
    fn path_between(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let edges = self.edges.borrow();
        let mut stack = vec![(from.to_string(), vec![from.to_string()])];
        let mut seen = BTreeSet::new();
        while let Some((node, path)) = stack.pop() {
            if node == to {
                return Some(path);
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            for next in edges.get(&node).into_iter().flatten() {
                let mut longer = path.clone();
                longer.push(next.clone());
                stack.push((next.clone(), longer));
            }
        }
        None
    }

    /// The requests the selected checkouts of `round` make, and the slots
    /// whose config is not on this machine to read.
    fn expand(&self, round: &Round) -> Result<(Vec<Req>, BTreeMap<SlotKey, String>)> {
        let mut requests = Vec::new();
        let mut wanted = Vec::new();
        let mut unexpanded = BTreeMap::new();
        for (key, state) in &round.slots {
            let Some(commit) = &state.commit else {
                continue;
            };
            if !self.expands(state) {
                continue;
            }
            let label = slot_label(state);
            self.labels
                .borrow_mut()
                .insert(key.url.clone(), label.clone());
            let config = match self.config_of(key, state, commit) {
                Ok(config) => config,
                Err(e) if is_unavailable(&e) => {
                    unexpanded.insert(key.clone(), e.to_string());
                    continue;
                }
                Err(e) => return Err(e),
            };
            let Some(config) = config else {
                continue;
            };
            for entry in &config.repos {
                let req = self.request(Node::Slot(key.clone()), entry);
                if !self.declared.contains(&req.url) && !self.allowed(&entry.repo_url) {
                    bail!(
                        "{} → {} wants {}, which is not on the allowlist for implicit \
                         dependencies; add \"{}\" to [resolve] allow in the root {}, or declare \
                         it at the root",
                        self.chain_text(round, &Node::Slot(key.clone())),
                        entry.directory,
                        entry.repo_url,
                        suggested_pattern(&entry.repo_url),
                        CONFIG_FILENAME
                    );
                }
                self.edge(&key.url, &req.url)?;
                wanted.push((entry.repo_url.clone(), req.kind));
                requests.push(req);
            }
        }
        self.repos.prepare(&wanted);
        Ok((requests, unexpanded))
    }

    /// Whether a slot's own dependencies are read: `recursive = false` on
    /// the root's entry says not, and an implicit slot is read unless every
    /// request for it says not.
    fn expands(&self, state: &SlotState) -> bool {
        match &state.root_entry {
            Some(entry) => entry.recursive,
            None => state.requests.iter().any(|r| r.entry.recursive),
        }
    }

    /// The config `key`'s checkout has at `commit`: from the checkout itself
    /// when it sits at that commit, else from the repository.
    fn config_of(
        &self,
        key: &SlotKey,
        state: &SlotState,
        commit: &str,
    ) -> Result<Option<GitScaleConfig>> {
        let cache_key = (key.url.clone(), commit.to_string(), state.directory.clone());
        if let Some(cached) = self.configs.borrow().get(&cache_key) {
            return cached.clone().map_err(|e| anyhow!(e));
        }
        let url = &state.requests[0].entry.repo_url;
        // A topic branch is source: its commits need not have an image, and
        // whatever the checkout holds, its dependencies are the source's.
        let kind = if state.topic.is_some() {
            Kind::Source
        } else {
            key.kind
        };
        let text = match self.checkouts.config_if_at(&state.directory, kind, commit) {
            Some(on_disk) => Ok(on_disk),
            None => match kind {
                Kind::Source => {
                    self.repos
                        .config_at(url, commit, state.chosen_revision().unwrap_or_default())
                }
                Kind::Artefact => self.repos.artefact_config(url, commit),
            },
        };
        let parsed = match text {
            Ok(Some(text)) => {
                let origin = PathBuf::from(format!(
                    "{}@{}:{}",
                    state.directory,
                    state
                        .chosen_revision()
                        .map(str::to_string)
                        .unwrap_or_else(|| crate::git::short_sha(commit).to_string()),
                    CONFIG_FILENAME
                ));
                parse_dependency_config(&text, &origin).map(Some)
            }
            Ok(None) => Ok(None),
            Err(e) if is_unavailable(&e) => {
                // Not cached: a later round, or the next command, may have it.
                return Err(e);
            }
            Err(e) => Err(e),
        };
        let stored = parsed
            .as_ref()
            .map(Clone::clone)
            .map_err(|e| format!("{:#}", e));
        self.configs.borrow_mut().insert(cache_key, stored);
        parsed
    }

    fn allowed(&self, url: &str) -> bool {
        let subject = crate::urls::normalize(url);
        self.allow
            .iter()
            .any(|pattern| crate::trust::glob_match(pattern, &subject))
    }

    /// Look a request's revision up, once per repository and revision.
    fn classify(&self, req: &mut Req) -> Result<()> {
        if req.entry.revision.is_empty() {
            req.info = Info::Empty;
            return Ok(());
        }
        let key = (req.url.clone(), req.entry.revision.clone());
        if let Some(cached) = self.classified.borrow().get(&key) {
            req.info = match cached {
                Classified::Known(info) => Info::Known(info.clone()),
                Classified::Missing(message) => Info::Missing(message.clone()),
                Classified::Failed(e) => bail!("{}", e),
            };
            return Ok(());
        }
        match self.lookup(&req.entry.repo_url, req.kind, &req.entry.revision) {
            Ok(info) => {
                self.classified
                    .borrow_mut()
                    .insert(key, Classified::Known(info.clone()));
                req.info = Info::Known(info);
            }
            Err(e) if is_unavailable(&e) => req.info = Info::Unavailable(e.to_string()),
            Err(e) if e.downcast_ref::<UnknownRevision>().is_some() => {
                self.repos.unknown(&req.entry.repo_url, req.kind);
                let message = e.to_string();
                self.classified
                    .borrow_mut()
                    .insert(key, Classified::Missing(message.clone()));
                req.info = Info::Missing(message);
            }
            Err(e) => {
                let message = format!("{:#}", e);
                self.classified
                    .borrow_mut()
                    .insert(key, Classified::Failed(message.clone()));
                bail!("{}", message);
            }
        }
        Ok(())
    }

    fn lookup(&self, url: &str, kind: Kind, revision: &str) -> Result<RevInfo> {
        let refs = self.repos.refs(url, kind)?;
        let branch = revision.strip_prefix("refs/heads/");
        let tag = revision.strip_prefix("refs/tags/");
        let (rev_kind, commit, tag_name) = if let Some(name) = branch {
            match refs.branches.get(name) {
                Some(commit) => (RevKind::Branch, commit.clone(), None),
                None => return Err(unknown(url, revision)),
            }
        } else if let Some(name) = tag {
            match refs.tags.get(name) {
                Some(commit) => (RevKind::Tag, commit.clone(), Some(name)),
                None => return Err(unknown(url, revision)),
            }
        } else if let Some(commit) = refs.branches.get(revision) {
            // A branch wins over a tag of the same name, as git has it.
            (RevKind::Branch, commit.clone(), None)
        } else if let Some(commit) = refs.tags.get(revision) {
            (RevKind::Tag, commit.clone(), Some(revision))
        } else if crate::git::is_full_sha(revision) {
            // Not checked here: that needs the commit, and reading its
            // config fetches it — failing then, with this revision named.
            (RevKind::Sha, revision.to_lowercase(), None)
        } else {
            return Err(unknown(url, revision));
        };
        let _ = kind;
        let version = tag_name.and_then(version::parse);
        let class = version
            .as_ref()
            .map(|v| if v.is_semver() { v.class() } else { Class::Any });
        Ok(RevInfo {
            commit,
            kind: rev_kind,
            version,
            class,
        })
    }

    /// Classify, group, select and place one round's requests.
    fn round(&self, mut requests: Vec<Req>, previous: Option<&Round>) -> Result<Round> {
        for req in &mut requests {
            self.classify(req).map_err(|e| {
                anyhow!(
                    "{} wants {} at {}: {:#}",
                    self.chain_text_opt(previous, &req.requester),
                    req.entry.repo_url,
                    req.entry.revision,
                    e
                )
            })?;
        }
        // Deterministic: the root's requests first, then by who asked.
        requests.sort_by(|a, b| {
            (a.requester != Node::Root, &a.requester, &a.entry.directory).cmp(&(
                b.requester != Node::Root,
                &b.requester,
                &b.entry.directory,
            ))
        });

        let singletons = self.singletons(&requests, previous);
        let mut slots: BTreeMap<SlotKey, Vec<Req>> = BTreeMap::new();
        let mut pending = Vec::new();
        let mut joining = Vec::new();
        for req in requests {
            let single = singletons.contains(&(req.url.clone(), req.kind));
            match req.known().map(|i| i.class) {
                Some(Some(class)) => {
                    let key = SlotKey {
                        url: req.url.clone(),
                        class: if single { Class::Any } else { class },
                        kind: req.kind,
                    };
                    slots.entry(key).or_default().push(req);
                }
                Some(None) if single => {
                    let key = SlotKey {
                        url: req.url.clone(),
                        class: Class::Any,
                        kind: req.kind,
                    };
                    slots.entry(key).or_default().push(req);
                }
                Some(None) => joining.push(req),
                None => pending.push(req),
            }
        }
        // A branch or commit joins the one checkout its repository's versions
        // are in — or makes the first, when nobody asks for a version.
        for req in joining {
            let classes: Vec<SlotKey> = slots
                .keys()
                .filter(|k| k.url == req.url && k.kind == req.kind)
                .cloned()
                .collect();
            let key = match classes.as_slice() {
                [] => SlotKey {
                    url: req.url.clone(),
                    class: Class::Any,
                    kind: req.kind,
                },
                [only] => only.clone(),
                many => bail!(
                    "{}, which is not a version, but {} is wanted at {}; which checkout it \
                     means is not clear. Ask for a version instead",
                    self.describe(previous, &req),
                    repo_name(&req.entry.repo_url),
                    many.iter()
                        .map(|k| format!("major {}", k.class))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            };
            slots.entry(key).or_default().push(req);
        }
        // No revision, or none to be found: the lowest class the repository
        // has, which is the checkout that keeps the plain name.
        for req in pending {
            let key = slots
                .keys()
                .find(|k| k.url == req.url && k.kind == req.kind)
                .cloned()
                .unwrap_or(SlotKey {
                    url: req.url.clone(),
                    class: Class::Any,
                    kind: req.kind,
                });
            slots.entry(key).or_default().push(req);
        }

        let dominators = dominators(&slots);
        let mut states = BTreeMap::new();
        for (key, reqs) in slots {
            let root_entry = self.root_entry_of(&key, &reqs)?;
            let single = singletons.contains(&(key.url.clone(), key.kind));
            let choice = self.select(&key, &reqs, &dominators, single, previous)?;
            states.insert(
                key,
                SlotState {
                    requests: reqs,
                    choice,
                    directory: String::new(),
                    root_entry,
                    commit: None,
                    unexpanded: None,
                    topic: None,
                    branch: None,
                    pinned_by: None,
                },
            );
        }
        let mut round = Round {
            slots: states,
            dominators,
        };
        self.place(&mut round, previous)?;
        self.apply_topic(&mut round, previous);
        for (key, state) in round.slots.iter_mut() {
            state.commit = match &state.choice {
                Choice::Picked { winner, .. } => {
                    state.requests[*winner].known().map(|i| i.commit.clone())
                }
                Choice::Follow => self.follow_commit(key, state),
                Choice::Unresolved(_) => None,
            };
        }
        Ok(round)
    }

    /// Put each slot that may be on the topic on its branch: the root
    /// store's own when there is one — work not pushed yet included — else
    /// the remote's.
    ///
    /// Held at its pin instead: a slot the root overrides, and one a
    /// requester keeps there by pinning the topic branch in its own config —
    /// or by being kept at its own pin that way, all the way down. A
    /// repository on its pinned branch builds its dependencies from pins, and
    /// the topic must not build it any other way.
    fn apply_topic(&self, round: &mut Round, previous: Option<&Round>) {
        let Some(topic) = &self.topic else {
            return;
        };
        let keys: Vec<SlotKey> = round.slots.keys().cloned().collect();
        for key in &keys {
            let highest = keys
                .iter()
                .filter(|k| k.url == key.url && k.kind == key.kind)
                .map(|k| k.class)
                .max()
                == Some(key.class);
            let pinned_by = self.pinned_by(round, previous, key, topic);
            let state = round.slots.get_mut(key).unwrap();
            if state.root_entry.as_ref().is_some_and(|e| e.is_override) {
                continue;
            }
            if let Some(by) = pinned_by {
                state.pinned_by = Some(by);
                continue;
            }
            let branch = branch_for(topic, highest, key.class);
            state.branch = Some(branch.clone());
            let url = state.requests[0].entry.repo_url.clone();
            let local = self.repos.local_branch(&url, &branch);
            let developed = local.is_some();
            let commit = match local {
                Some(commit) => commit,
                None => match self.repos.refs(&url, key.kind) {
                    Ok(refs) => match refs.branches.get(&branch) {
                        Some(commit) => commit.clone(),
                        None => continue,
                    },
                    Err(_) => continue,
                },
            };
            let first = &state.requests[0].entry;
            let entry = RepoEntry {
                directory: state.directory.clone(),
                repo_url: state
                    .root_entry
                    .as_ref()
                    .map_or(first.repo_url.clone(), |e| e.repo_url.clone()),
                revision: branch.clone(),
                artefact: state
                    .root_entry
                    .as_ref()
                    .map_or(first.artefact, |e| e.artefact),
                ..RepoEntry::default()
            };
            let pinned = match &state.choice {
                Choice::Picked { winner, .. } => Some(*winner),
                _ => None,
            };
            state.requests.push(Req {
                requester: Node::Root,
                url: key.url.clone(),
                kind: key.kind,
                entry,
                info: Info::Known(RevInfo {
                    commit,
                    kind: RevKind::Branch,
                    version: None,
                    class: None,
                }),
                topic: true,
            });
            state.choice = Choice::Picked {
                winner: state.requests.len() - 1,
                reason: Reason::Topic,
                resolution: "topic",
                overruled: Vec::new(),
                ahead: Vec::new(),
            };
            state.topic = Some(TopicPick { developed, pinned });
        }
    }

    /// The requester of `key` that keeps it at its pin: one whose own config
    /// pins `topic`, or one kept at its pin itself. Judged on the previous
    /// round, whose configs are the ones read; pinned wins between requesters.
    fn pinned_by(
        &self,
        round: &Round,
        previous: Option<&Round>,
        key: &SlotKey,
        topic: &str,
    ) -> Option<String> {
        let previous = previous?;
        for req in &round.slots[key].requests {
            let Node::Slot(requester) = &req.requester else {
                continue;
            };
            let Some(state) = previous.slots.get(requester) else {
                continue;
            };
            if state.pinned_by.is_some() {
                return state.pinned_by.clone();
            }
            let Some(commit) = &state.commit else {
                continue;
            };
            let pins = self
                .configs
                .borrow()
                .get(&(
                    requester.url.clone(),
                    commit.clone(),
                    state.directory.clone(),
                ))
                .and_then(|c| c.as_ref().ok().cloned())
                .flatten()
                .and_then(|c| c.develop.pinned)
                .is_some_and(|patterns| crate::topic::pins(&patterns, topic));
            if pins {
                return Some(state.directory.clone());
            }
        }
        None
    }

    /// The commit a slot nobody gives a revision is read at and checked out
    /// at: the remote's default branch.
    fn follow_commit(&self, key: &SlotKey, state: &SlotState) -> Option<String> {
        let url = &state.requests[0].entry.repo_url;
        let branch = self.repos.default_branch(url, key.kind).ok()??;
        self.repos
            .refs(url, key.kind)
            .ok()?
            .branches
            .get(&branch)
            .cloned()
    }

    /// The root entry a slot belongs to, if the root declares it.
    fn root_entry_of(&self, key: &SlotKey, reqs: &[Req]) -> Result<Option<RepoEntry>> {
        let roots: Vec<&Req> = reqs.iter().filter(|r| r.requester == Node::Root).collect();
        match roots.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some(only.entry.clone())),
            [a, b, ..] => bail!(
                "repos.{} and repos.{} in the root {} are both checkouts of {} (class {}); \
                 give each a revision of a different major, or remove one",
                a.entry.directory,
                b.entry.directory,
                CONFIG_FILENAME,
                a.entry.repo_url,
                key.class
            ),
        }
    }

    /// The URLs that may be checked out only once, by what the requests and
    /// the repositories' own configs say — with `singleton = false` from a
    /// repository relaxing what the repositories it dominates say.
    fn singletons(&self, requests: &[Req], previous: Option<&Round>) -> BTreeSet<(String, Kind)> {
        let empty = Dominators::default();
        let dominators = previous.map_or(&empty, |p| &p.dominators);
        let mut trues: BTreeMap<(String, Kind), Vec<Node>> = BTreeMap::new();
        let mut falses: BTreeMap<(String, Kind), Vec<Node>> = BTreeMap::new();
        for req in requests {
            let key = (req.url.clone(), req.kind);
            match req.entry.singleton {
                Some(true) => trues.entry(key).or_default().push(req.requester.clone()),
                Some(false) => falses.entry(key).or_default().push(req.requester.clone()),
                None => {}
            }
        }
        // A repository that says so of itself, at the revision selected.
        if let Some(round) = previous {
            for (key, state) in &round.slots {
                let Some(commit) = &state.commit else {
                    continue;
                };
                let config = self
                    .configs
                    .borrow()
                    .get(&(key.url.clone(), commit.clone(), state.directory.clone()))
                    .and_then(|c| c.as_ref().ok().cloned())
                    .flatten();
                if config.is_some_and(|c| c.singleton) {
                    trues
                        .entry((key.url.clone(), key.kind))
                        .or_default()
                        .push(Node::Slot(key.clone()));
                }
            }
        }
        trues
            .into_iter()
            .filter(|(key, nodes)| {
                let relaxers = falses.get(key).cloned().unwrap_or_default();
                nodes
                    .iter()
                    .any(|t| !relaxers.iter().any(|f| dominators.dominates(f, t)))
            })
            .map(|(key, _)| key)
            .collect()
    }

    fn select(
        &self,
        key: &SlotKey,
        reqs: &[Req],
        dom: &Dominators,
        singleton: bool,
        previous: Option<&Round>,
    ) -> Result<Choice> {
        if let Some(Info::Unavailable(message)) = reqs
            .iter()
            .map(|r| &r.info)
            .find(|i| matches!(i, Info::Unavailable(_)))
        {
            return Ok(Choice::Unresolved(message.clone()));
        }
        let known: Vec<usize> = (0..reqs.len())
            .filter(|&i| reqs[i].known().is_some())
            .collect();
        let describe = |i: usize| self.describe(previous, &reqs[i]);
        let result = (|| -> Result<Choice> {
            let overrides: Vec<usize> = known
                .iter()
                .copied()
                .filter(|&i| reqs[i].entry.is_override)
                .collect();
            // The outer of two overrides wins; those nothing outranks remain.
            let effective: Vec<usize> = overrides
                .iter()
                .copied()
                .filter(|&o| {
                    !overrides.iter().any(|&p| {
                        p != o
                            && reqs[p].requester != reqs[o].requester
                            && dom.dominates(&reqs[p].requester, &reqs[o].requester)
                    })
                })
                .collect();
            // A revision the repository does not have is a mistake — unless
            // an override from a repository above the one that made it
            // replaces it anyway, which is how a root works around a broken
            // pin in a dependency it cannot edit.
            let mut missing = Vec::new();
            for (m, req) in reqs.iter().enumerate() {
                let Info::Missing(message) = &req.info else {
                    continue;
                };
                let covered = effective.iter().copied().find(|&o| {
                    reqs[o].requester != req.requester
                        && dom.dominates(&reqs[o].requester, &req.requester)
                });
                let Some(by) = covered else {
                    bail!(
                        "{} wants {} at {}: {}",
                        self.chain_text_opt(previous, &req.requester),
                        req.entry.repo_url,
                        req.entry.revision,
                        message
                    );
                };
                missing.push((m, by));
            }
            if known.is_empty() {
                return Ok(Choice::Follow);
            }
            for &other in effective.iter().skip(1) {
                let (a, b) = (
                    reqs[effective[0]].known().unwrap(),
                    reqs[other].known().unwrap(),
                );
                if a.commit != b.commit {
                    bail!(
                        "conflicting overrides: {} and {}. Neither repository dominates the \
                         other; an override in one that dominates both, such as the root, \
                         settles it",
                        describe(effective[0]),
                        describe(other)
                    );
                }
            }
            if let Some(&o) = effective.first() {
                let mut overruled = missing;
                for &i in &known {
                    if i == o || effective.contains(&i) {
                        continue;
                    }
                    // Tied overrides all name one commit: each wins over what
                    // its own repository is above.
                    let above = effective
                        .iter()
                        .copied()
                        .find(|&e| dom.dominates(&reqs[e].requester, &reqs[i].requester));
                    if let Some(by) = above {
                        let higher = !matches!(
                            self.compare(&reqs[by], &reqs[i], dom),
                            Ok((Ordering::Greater | Ordering::Equal, _))
                        );
                        if higher {
                            overruled.push((i, by));
                        }
                        continue;
                    }
                    if singleton && different_majors(&reqs[i], &reqs[o]) {
                        bail!(
                            "{} is a singleton, but {} and {} want different majors",
                            reqs[o].entry.repo_url,
                            describe(o),
                            describe(i)
                        );
                    }
                    let (order, _) = self.compare(&reqs[o], &reqs[i], dom)?;
                    if order == Ordering::Less {
                        bail!(
                            "override conflict: {} (override), but {} needs more, and {} does \
                             not dominate it. An override in a repository that dominates both, \
                             such as the root, settles it",
                            describe(o),
                            describe(i),
                            requester_name(previous, &reqs[o].requester)
                        );
                    }
                }
                return Ok(Choice::Picked {
                    winner: o,
                    reason: Reason::Override,
                    resolution: "override",
                    overruled,
                    ahead: self.ahead_of(key, reqs, o, &known),
                });
            }

            if singleton {
                if let Some(&i) = known
                    .iter()
                    .find(|&&i| different_majors(&reqs[i], &reqs[known[0]]))
                {
                    bail!(
                        "{} is a singleton, but {} and {} want different majors. An override \
                         at the root settles it, or singleton = false on the root's entry",
                        reqs[i].entry.repo_url,
                        describe(known[0]),
                        describe(i)
                    );
                }
            }
            let mut best = known[0];
            let mut reason = Reason::Only;
            for &i in &known[1..] {
                let (order, why) = self.compare(&reqs[best], &reqs[i], dom)?;
                match order {
                    Ordering::Less => {
                        best = i;
                        reason = why;
                    }
                    Ordering::Equal if reason == Reason::Only => reason = Reason::Equal,
                    Ordering::Greater if reason == Reason::Only || reason == Reason::Equal => {
                        reason = why
                    }
                    _ => {}
                }
            }
            // Pairwise picks can disagree once versions and history mix;
            // the winner has to be at least every request, or nothing is.
            for &i in &known {
                let (order, _) = self.compare(&reqs[best], &reqs[i], dom)?;
                if order == Ordering::Less {
                    bail!("cannot order {} against {}", describe(best), describe(i));
                }
            }
            let root_request = known
                .iter()
                .copied()
                .find(|&i| reqs[i].requester == Node::Root);
            let resolution = if known.len() == 1 {
                "only"
            } else {
                match root_request {
                    Some(r) if r == best => "root",
                    Some(r)
                        if reqs[r].known().unwrap().commit
                            == reqs[best].known().unwrap().commit =>
                    {
                        "root"
                    }
                    Some(_) => "raised",
                    None => "highest",
                }
            };
            let best = match root_request {
                // The root's own spelling, when it names the same commit.
                Some(r) if resolution == "root" => r,
                _ => best,
            };
            Ok(Choice::Picked {
                winner: best,
                reason,
                resolution,
                overruled: Vec::new(),
                ahead: self.ahead_of(key, reqs, best, &known),
            })
        })();
        match result {
            Err(e) if is_unavailable(&e) => Ok(Choice::Unresolved(e.to_string())),
            other => other,
        }
    }

    /// How two requests in one slot order: as versions when both are, in
    /// one stream; otherwise by position, the request from the repository
    /// that dominates the other winning. No history is read: the answer is
    /// the same on a shallow CI checkout as on a full mirror.
    fn compare(&self, a: &Req, b: &Req, dom: &Dominators) -> Result<(Ordering, Reason)> {
        let (ia, ib) = (a.known().unwrap(), b.known().unwrap());
        if a.entry.revision == b.entry.revision {
            return Ok((Ordering::Equal, Reason::Equal));
        }
        if let (Some(va), Some(vb)) = (&ia.version, &ib.version) {
            if let Some(order) = va.compare(vb) {
                let reason = if va.is_semver() {
                    Reason::Semver
                } else {
                    Reason::Calver
                };
                return Ok((order, reason));
            }
        }
        if ia.commit == ib.commit {
            return Ok((Ordering::Equal, Reason::Equal));
        }
        if a.requester != b.requester {
            if dom.dominates(&a.requester, &b.requester) {
                return Ok((Ordering::Greater, Reason::Position));
            }
            if dom.dominates(&b.requester, &a.requester) {
                return Ok((Ordering::Less, Reason::Position));
            }
        }
        bail!(
            "cannot order {} against {}: they are not versions of one stream, and neither \
             repository is above the other. Ask for versions in both, or set the revision in a \
             repository above both, such as the root",
            a.entry.revision,
            b.entry.revision
        )
    }

    /// The requests asking for a commit `winner` turns out to be behind:
    /// only those compared by position, and only where history is on local
    /// disk. A warning; it changes nothing.
    fn ahead_of(&self, key: &SlotKey, reqs: &[Req], winner: usize, known: &[usize]) -> Vec<usize> {
        let w = reqs[winner].known().unwrap();
        known
            .iter()
            .copied()
            .filter(|&i| {
                let r = reqs[i].known().unwrap();
                i != winner
                    && r.commit != w.commit
                    && !matches!((&w.version, &r.version), (Some(a), Some(b)) if a.compare(b).is_some())
                    && matches!(
                        self.repos.is_ancestor(&reqs[i].entry.repo_url, key.kind, &w.commit, &r.commit),
                        Ok(true)
                    )
            })
            .collect()
    }

    /// Give every slot its directory: the root's, or one under the hoist
    /// directory named after what the requesters called it.
    fn place(&self, round: &mut Round, previous: Option<&Round>) -> Result<()> {
        let hoist = Path::new(&self.config.resolve.hoist_dir);
        let keys: Vec<SlotKey> = round.slots.keys().cloned().collect();
        // Where the root put its own checkouts: an implicit one of the same
        // repository must not land on one of them.
        let declared: BTreeSet<String> = round
            .slots
            .values()
            .filter_map(|s| s.root_entry.as_ref().map(|e| e.directory.clone()))
            .collect();
        for key in &keys {
            let lowest = keys
                .iter()
                .filter(|k| k.url == key.url && k.kind == key.kind)
                .map(|k| k.class)
                .min()
                == Some(key.class);
            let has_source = keys
                .iter()
                .any(|k| k.url == key.url && k.kind == Kind::Source);
            let state = round.slots.get_mut(key).unwrap();
            if let Some(entry) = &state.root_entry {
                state.directory = entry.directory.clone();
                continue;
            }
            let names: BTreeSet<String> = state
                .requests
                .iter()
                .filter_map(|r| {
                    Path::new(&r.entry.directory)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .collect();
            let mut name = match names.len() {
                1 => names.into_iter().next().unwrap(),
                _ => repo_name(&state.requests[0].entry.repo_url),
            };
            if !lowest {
                name.push_str(&key.class.suffix());
            }
            if key.kind == Kind::Artefact && has_source {
                name.push_str("_artefact");
            }
            let mut directory = hoist.join(&name).to_string_lossy().into_owned();
            // The plain name is the root's checkout of another major — the
            // root declared v2 at `imports/mylib` and a dependency wants v1.
            if declared.contains(&directory) {
                let suffix = match key.class.suffix() {
                    s if s.is_empty() => format!("_{}", key.class),
                    s => s,
                };
                directory = hoist
                    .join(format!("{}{}", name, suffix))
                    .to_string_lossy()
                    .into_owned();
            }
            state.directory = directory;
        }

        let mut taken: BTreeMap<String, &SlotKey> = BTreeMap::new();
        for (key, state) in &round.slots {
            if let Some(other) = taken.insert(state.directory.clone(), key) {
                let other_state = &round.slots[other];
                bail!(
                    "two repositories want {}: {} (wanted by {}) and {} (wanted by {}). Declare \
                     one of them at the root under another directory",
                    state.directory,
                    other_state.requests[0].entry.repo_url,
                    self.describe(previous, &other_state.requests[0]),
                    state.requests[0].entry.repo_url,
                    self.describe(previous, &state.requests[0]),
                );
            }
        }
        Ok(())
    }

    /// `root → imports/b@v2.1.0 wants proto v1.4.0`.
    fn describe(&self, previous: Option<&Round>, req: &Req) -> String {
        let revision = if req.entry.revision.is_empty() {
            "(any revision)".to_string()
        } else {
            req.entry.revision.clone()
        };
        format!(
            "{} wants {} {}",
            self.chain_text_opt(previous, &req.requester),
            repo_name(&req.entry.repo_url),
            revision
        )
    }

    fn chain_text_opt(&self, previous: Option<&Round>, node: &Node) -> String {
        match previous {
            Some(round) => self.chain_text(round, node),
            None => "root".to_string(),
        }
    }

    /// `root → imports/b@v2.1.0`: the checkouts a request came through.
    fn chain_text(&self, round: &Round, node: &Node) -> String {
        std::iter::once("root".to_string())
            .chain(chain(round, node))
            .collect::<Vec<_>>()
            .join(" → ")
    }

    /// The answer, once a round changes nothing.
    fn finish(&self, round: Round) -> Result<Resolution> {
        let mut slots = Vec::new();
        let mut links = Vec::new();
        for (key, state) in &round.slots {
            let majors = round
                .slots
                .keys()
                .filter(|k| k.url == key.url && k.kind == key.kind)
                .count();
            let requests: Vec<RequestView> = state
                .requests
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.topic)
                .map(|(i, r)| {
                    let (selected, overruled, ahead) = match &state.choice {
                        Choice::Picked {
                            winner,
                            overruled,
                            ahead,
                            ..
                        } => (
                            *winner == i,
                            overruled.iter().find(|(o, _)| *o == i).map(|(_, by)| *by),
                            ahead.contains(&i),
                        ),
                        _ => (false, None, false),
                    };
                    RequestView {
                        from: requester_name(Some(&round), &r.requester),
                        chain: chain(&round, &r.requester),
                        directory: r.entry.directory.clone(),
                        revision: r.entry.revision.clone(),
                        revision_kind: r.known().map(|i| i.kind),
                        commit: r.known().map(|i| i.commit.clone()),
                        is_override: r.entry.is_override,
                        selected,
                        overruled_by: overruled
                            .map(|by| requester_name(Some(&round), &state.requests[by].requester)),
                        ahead,
                        missing: matches!(r.info, Info::Missing(_)),
                    }
                })
                .collect();
            let chosen = match &state.choice {
                Choice::Picked {
                    winner,
                    reason,
                    resolution,
                    ..
                } => {
                    let req = &state.requests[*winner];
                    let info = req.known().unwrap();
                    Some(Chosen {
                        revision: req.entry.revision.clone(),
                        commit: info.commit.clone(),
                        kind: info.kind,
                        reason: *reason,
                        resolution,
                        by: if req.topic {
                            "topic".to_string()
                        } else {
                            requester_name(Some(&round), &req.requester)
                        },
                    })
                }
                _ => None,
            };
            let unresolved = match &state.choice {
                Choice::Unresolved(message) => Some(message.clone()),
                _ => None,
            };
            let class = state.choice_class().unwrap_or(key.class);
            let first = &state.requests[0].entry;
            slots.push(Slot {
                directory: state.directory.clone(),
                url: state
                    .root_entry
                    .as_ref()
                    .map_or(first.repo_url.clone(), |e| e.repo_url.clone()),
                artefact: match (&state.root_entry, key.kind) {
                    (Some(entry), _) => entry.artefact,
                    (None, Kind::Artefact) => Some(ArtefactUse::Replace),
                    // Overlaid when any repository asks for it overlaid.
                    (None, Kind::Source) => state
                        .requests
                        .iter()
                        .any(|r| r.entry.is_overlay())
                        .then_some(ArtefactUse::Overlay),
                },
                recursive: self.expands(state),
                kind: key.kind,
                class,
                declared: state.root_entry.as_ref().map(|e| e.revision.clone()),
                implicit: state.root_entry.is_none(),
                chosen,
                unresolved,
                unread: state.unexpanded.clone(),
                requests,
                majors,
                commit: state.commit.clone(),
                branch: state.branch.clone(),
                topic: state.topic.as_ref().and_then(|pick| {
                    let Choice::Picked { winner, .. } = &state.choice else {
                        return None;
                    };
                    let req = &state.requests[*winner];
                    Some(SlotTopic {
                        branch: req.entry.revision.clone(),
                        commit: req.known()?.commit.clone(),
                        developed: pick.developed,
                        pin: pick.pinned.and_then(|p| {
                            let pinned = &state.requests[p];
                            Some(Pin {
                                revision: pinned.entry.revision.clone(),
                                commit: pinned.known()?.commit.clone(),
                                by: requester_name(Some(&round), &pinned.requester),
                            })
                        }),
                    })
                }),
                pinned_by: state.pinned_by.clone(),
            });
        }
        // Each request a checkout makes is a link inside that checkout, to
        // the one checkout of what it asked for.
        for target in round.slots.values() {
            for req in &target.requests {
                let Node::Slot(holder_key) = &req.requester else {
                    continue;
                };
                let Some(holder) = round.slots.get(holder_key) else {
                    continue;
                };
                let link_path = Path::new(&holder.directory).join(&req.entry.directory);
                let target_path = PathBuf::from(&target.directory);
                if link_path == target_path {
                    continue;
                }
                links.push(SymlinkEntry {
                    link_path,
                    target_path,
                });
            }
        }
        links.sort_by(|a, b| a.link_path.cmp(&b.link_path));
        links.dedup_by(|a, b| a.link_path == b.link_path);

        // The root's entries as the config lists them once read (sorted by
        // directory), then the implicit slots by path.
        let order: Vec<&str> = self
            .config
            .repos
            .iter()
            .map(|e| e.directory.as_str())
            .collect();
        slots.sort_by(|a, b| {
            let pos = |s: &Slot| {
                order
                    .iter()
                    .position(|d| *d == s.directory)
                    .unwrap_or(usize::MAX)
            };
            pos(a)
                .cmp(&pos(b))
                .then_with(|| a.directory.cmp(&b.directory))
        });
        Ok(Resolution { slots, links })
    }
}

impl SlotState {
    fn choice_class(&self) -> Option<Class> {
        match &self.choice {
            Choice::Picked { winner, .. } => self.requests[*winner].known().and_then(|i| i.class),
            _ => None,
        }
    }
}

/// Whether two requests are versions of different majors. A branch or a
/// commit has no major of its own, and differs from nothing.
fn different_majors(a: &Req, b: &Req) -> bool {
    match (
        a.known().and_then(|i| i.class),
        b.known().and_then(|i| i.class),
    ) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}

/// What changes between rounds: which slots there are, where, and at what.
type Fingerprint = (String, String, Option<String>, bool, Option<String>);

fn fingerprint(round: &Round) -> BTreeMap<SlotKey, Fingerprint> {
    round
        .slots
        .iter()
        .map(|(key, state)| {
            (
                key.clone(),
                (
                    state.directory.clone(),
                    state.chosen_revision().unwrap_or_default().to_string(),
                    state.commit.clone(),
                    matches!(state.choice, Choice::Unresolved(_)),
                    state.pinned_by.clone(),
                ),
            )
        })
        .collect()
}

/// The branch of the topic `topic` a checkout uses: the topic itself for the
/// highest major its repository is checked out at, `<topic>@v<major>` for the
/// others — two worktrees of one store cannot share a branch, and a name is
/// what CI matches on.
pub fn branch_for(topic: &str, highest: bool, class: Class) -> String {
    if highest || class == Class::Any {
        topic.to_string()
    } else {
        format!("{}@v{}", topic, class.describe())
    }
}

/// `imports/b@v2.1.0`.
fn slot_label(state: &SlotState) -> String {
    let revision = match (state.chosen_revision(), &state.commit) {
        (Some(revision), _) => revision.to_string(),
        (None, Some(commit)) => crate::git::short_sha(commit).to_string(),
        (None, None) => "?".to_string(),
    };
    format!("{}@{}", state.directory, revision)
}

/// `root`, or a requesting checkout's directory.
fn requester_name(round: Option<&Round>, node: &Node) -> String {
    match node {
        Node::Root => "root".to_string(),
        Node::Slot(key) => round
            .and_then(|r| r.slots.get(key))
            .map(|s| s.directory.clone())
            .unwrap_or_else(|| key.url.clone()),
    }
}

/// The labels of the checkouts between the root and `node`, `node` included,
/// by the way each was first asked for.
fn chain(round: &Round, node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = node.clone();
    let mut seen = BTreeSet::new();
    while let Node::Slot(key) = &current {
        if !seen.insert(key.clone()) {
            break;
        }
        let Some(state) = round.slots.get(key) else {
            out.push(key.url.clone());
            break;
        };
        out.push(slot_label(state));
        let parent = match &state.choice {
            Choice::Picked { winner, .. } => state.requests[*winner].requester.clone(),
            _ => state.requests[0].requester.clone(),
        };
        current = parent;
    }
    out.reverse();
    out
}

/// Dominators of the request graph: requester → the slot it asked for.
fn dominators(slots: &BTreeMap<SlotKey, Vec<Req>>) -> Dominators {
    let mut preds: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    for (key, reqs) in slots {
        let node = Node::Slot(key.clone());
        for req in reqs {
            if req.requester != node {
                preds
                    .entry(node.clone())
                    .or_default()
                    .insert(req.requester.clone());
            }
        }
    }
    let mut dom: HashMap<Node, BTreeSet<Node>> = HashMap::new();
    dom.insert(Node::Root, BTreeSet::from([Node::Root]));
    let all: BTreeSet<Node> = std::iter::once(Node::Root)
        .chain(slots.keys().map(|k| Node::Slot(k.clone())))
        .collect();
    // Iterate to a fixed point: the graph has no cycles, so this is a
    // handful of passes.
    for node in all.iter().filter(|n| **n != Node::Root) {
        dom.insert(node.clone(), all.clone());
    }
    loop {
        let mut changed = false;
        for node in all.iter().filter(|n| **n != Node::Root) {
            let mut set: Option<BTreeSet<Node>> = None;
            for pred in preds.get(node).into_iter().flatten() {
                // A requester that is not a slot this round contributes
                // nothing but itself and the root.
                let pred_dom = dom
                    .get(pred)
                    .cloned()
                    .unwrap_or_else(|| BTreeSet::from([pred.clone(), Node::Root]));
                set = Some(match set {
                    None => pred_dom,
                    Some(s) => s.intersection(&pred_dom).cloned().collect(),
                });
            }
            let mut set = set.unwrap_or_else(|| BTreeSet::from([Node::Root]));
            set.insert(node.clone());
            if dom.get(node) != Some(&set) {
                dom.insert(node.clone(), set);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Dominators { dom }
}

fn unknown(url: &str, revision: &str) -> anyhow::Error {
    let message = crate::git::with_revision_hint(
        revision,
        anyhow!("'{}' is not a branch, tag or commit of {}", revision, url),
    );
    anyhow::Error::new(UnknownRevision(format!("{:#}", message)))
}

/// The repository's own name: the last part of its URL, without `.git`.
pub fn repo_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

/// The patterns implicit dependencies are allowed by: the host and top-level
/// owner of every repository the root declares, and of the root itself, plus
/// `[resolve] allow`. Only the root's config feeds it, so trust does not grow
/// down the graph; local paths are never allowed this way.
fn allowlist(config: &GitScaleConfig, root_url: Option<&str>) -> Vec<String> {
    let mut patterns: Vec<String> = config
        .repos
        .iter()
        .map(|e| e.repo_url.as_str())
        .chain(root_url)
        .filter_map(owner_pattern)
        .collect();
    patterns.extend(config.resolve.allow.iter().cloned());
    patterns.sort();
    patterns.dedup();
    patterns
}

/// `github.com/org/*` for any repository of `org` on `github.com`, nested
/// groups included.
fn owner_pattern(url: &str) -> Option<String> {
    let host = crate::urls::extract_hostname(url).ok()?;
    let (owner, _) = crate::urls::extract_owner_repo(url).ok()?;
    Some(format!("{}/{}/*", host, owner).to_lowercase())
}

/// What to add to `[resolve] allow` for `url`.
fn suggested_pattern(url: &str) -> String {
    owner_pattern(url).unwrap_or_else(|| crate::urls::normalize(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;
    use std::collections::BTreeMap;

    /// A repository: branches and tags, the parent of each commit, and the
    /// config at each commit.
    #[derive(Default, Clone)]
    struct FakeRepo {
        branches: BTreeMap<String, String>,
        tags: BTreeMap<String, String>,
        parents: BTreeMap<String, String>,
        configs: BTreeMap<String, String>,
    }

    #[derive(Default)]
    struct Fake {
        repos: BTreeMap<String, FakeRepo>,
        /// Whether history can be read: a developer machine's stores.
        history: bool,
        /// The workspace's own branches, by repository: where topics are
        /// developed.
        local: BTreeMap<String, BTreeMap<String, String>>,
    }

    impl Fake {
        fn repo(&mut self, url: &str) -> &mut FakeRepo {
            self.repos.entry(url.to_string()).or_default()
        }

        fn get(&self, url: &str) -> Result<&FakeRepo> {
            self.repos
                .get(url)
                .ok_or_else(|| anyhow!("no such repository {}", url))
        }
    }

    impl FakeRepo {
        /// A commit `sha` with `parent`, tagged and holding `config`.
        fn commit(&mut self, sha: &str, parent: Option<&str>, config: Option<&str>) -> &mut Self {
            if let Some(parent) = parent {
                self.parents.insert(sha.to_string(), parent.to_string());
            }
            if let Some(config) = config {
                self.configs.insert(sha.to_string(), config.to_string());
            }
            self
        }

        fn tag(&mut self, name: &str, sha: &str) -> &mut Self {
            self.tags.insert(name.to_string(), sha.to_string());
            self
        }

        fn branch(&mut self, name: &str, sha: &str) -> &mut Self {
            self.branches.insert(name.to_string(), sha.to_string());
            self
        }

        fn history(&self, commit: &str) -> Vec<String> {
            let mut out = vec![commit.to_string()];
            let mut current = commit.to_string();
            while let Some(parent) = self.parents.get(&current) {
                out.push(parent.clone());
                current = parent.clone();
            }
            out
        }
    }

    impl Repos for Fake {
        fn refs(&self, url: &str, _: Kind) -> Result<Refs> {
            let repo = self.get(url)?;
            Ok(Refs {
                branches: repo.branches.clone(),
                tags: repo.tags.clone(),
                default_branch: Some("main".to_string()),
            })
        }
        fn config_at(&self, url: &str, commit: &str, _: &str) -> Result<Option<String>> {
            Ok(self.get(url)?.configs.get(commit).cloned())
        }
        fn artefact_config(&self, url: &str, commit: &str) -> Result<Option<String>> {
            self.config_at(url, commit, "")
        }
        fn default_branch(&self, _: &str, _: Kind) -> Result<Option<String>> {
            Ok(Some("main".to_string()))
        }
        fn is_ancestor(&self, url: &str, _: Kind, a: &str, b: &str) -> Result<bool> {
            if !self.history {
                return Err(unavailable("no history"));
            }
            Ok(self.get(url)?.history(b).iter().any(|c| c == a))
        }
        fn local_branch(&self, url: &str, branch: &str) -> Option<String> {
            self.local.get(url)?.get(branch).cloned()
        }
    }

    struct NoCheckouts;

    impl Checkouts for NoCheckouts {
        fn config_if_at(&self, _: &str, _: Kind, _: &str) -> Option<Option<String>> {
            None
        }
        fn head(&self, _: &str) -> Option<String> {
            None
        }
    }

    const D: &str = "https://github.com/org/d.git";
    const B: &str = "https://github.com/org/b.git";
    const C: &str = "https://github.com/org/c.git";
    const E: &str = "https://github.com/org/e.git";

    fn deps(entries: &[(&str, &str, &str)]) -> String {
        let mut text = String::from("[repos]\n");
        for (dir, url, rest) in entries {
            text.push_str(&format!("\"{}\" = {{ url = \"{}\"{} }}\n", dir, url, rest));
        }
        text
    }

    fn root(text: &str) -> GitScaleConfig {
        parse_config(text, Path::new("root/.gitscale.toml")).unwrap()
    }

    fn resolve(fake: &Fake, config: &str) -> Result<Resolution> {
        let config = root(config);
        Engine::new(&config, None, fake, &NoCheckouts).resolve()
    }

    /// D with v1.2.0 < v1.3.1 < v1.5.0 < v1.6.0 on one line, and v2.0.0.
    fn d_versions(fake: &mut Fake) {
        let d = fake.repo(D);
        d.commit("d120", None, None)
            .commit("d131", Some("d120"), None)
            .commit("d150", Some("d131"), None)
            .commit("d160", Some("d150"), None)
            .commit("d200", Some("d160"), None)
            .tag("v1.2.0", "d120")
            .tag("v1.3.1", "d131")
            .tag("v1.5.0", "d150")
            .tag("v1.6.0", "d160")
            .tag("v2.0.0", "d200")
            .branch("main", "d200");
    }

    fn child(fake: &mut Fake, url: &str, sha: &str, tag: &str, config: &str) {
        fake.repo(url)
            .commit(sha, None, Some(config))
            .tag(tag, sha)
            .branch("main", sha);
    }

    fn slot<'a>(resolution: &'a Resolution, directory: &str) -> &'a Slot {
        resolution
            .slot(directory)
            .unwrap_or_else(|| panic!("no slot {} in {:#?}", directory, resolution.slots))
    }

    #[test]
    fn the_highest_request_wins_and_the_root_takes_part() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v2.1.0",
            &deps(&[("libs/d", D, ", revision = \"v1.3.1\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v0.9.0",
            &deps(&[("vendor/d", D, ", revision = \"v1.5.0\"")]),
        );
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/b", B, ", revision = \"v2.1.0\""),
                ("imports/c", C, ", revision = \"v0.9.0\""),
                ("imports/d", D, ", revision = \"v1.2.0\""),
            ]),
        )
        .unwrap();
        let d = slot(&r, "imports/d");
        let chosen = d.chosen.as_ref().unwrap();
        assert_eq!(chosen.revision, "v1.5.0");
        assert_eq!(chosen.resolution, "raised");
        assert_eq!(chosen.reason, Reason::Semver);
        assert_eq!(chosen.by, "imports/c");
        assert_eq!(d.requests.len(), 3);
        let links: Vec<String> = r
            .links
            .iter()
            .map(|l| l.link_path.display().to_string())
            .collect();
        assert_eq!(links, vec!["imports/b/libs/d", "imports/c/vendor/d"]);
    }

    #[test]
    fn an_undeclared_dependency_is_placed_under_the_hoist_dir() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("vendor/shared", D, ", revision = \"v1.3.1\"")]),
        );
        let r = resolve(&fake, &deps(&[("imports/b", B, ", revision = \"v1.0.0\"")])).unwrap();
        let d = slot(&r, "imports/shared");
        assert!(d.implicit);
        assert_eq!(d.artefact, None);
        assert_eq!(d.chosen.as_ref().unwrap().revision, "v1.3.1");
        assert_eq!(r.links[0].target_path, PathBuf::from("imports/shared"));
    }

    #[test]
    fn two_majors_are_placed_side_by_side_and_singleton_refuses() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/mylib", D, ", revision = \"v1.3.1\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/mylib", D, ", revision = \"v2.0.0\"")]),
        );
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/c", C, ", revision = \"v1.0.0\""),
        ]);
        let r = resolve(&fake, &config).unwrap();
        assert_eq!(
            slot(&r, "imports/mylib").chosen.as_ref().unwrap().revision,
            "v1.3.1"
        );
        assert_eq!(
            slot(&r, "imports/mylib_v2")
                .chosen
                .as_ref()
                .unwrap()
                .revision,
            "v2.0.0"
        );
        assert_eq!(slot(&r, "imports/mylib").majors, 2);

        // The repository says of itself it may be checked out once.
        let mut fake = Fake::default();
        d_versions(&mut fake);
        for sha in ["d131", "d200"] {
            fake.repo(D)
                .configs
                .insert(sha.to_string(), "singleton = true\n".to_string());
        }
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/mylib", D, ", revision = \"v1.3.1\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/mylib", D, ", revision = \"v2.0.0\"")]),
        );
        let err = resolve(&fake, &config).unwrap_err().to_string();
        assert!(err.contains("singleton"), "{}", err);

        // The root relaxes it.
        let relaxed = format!(
            "{}\"imports/mylib\" = {{ url = \"{}\", revision = \"v1.3.1\", singleton = false }}\n",
            config, D
        );
        let r = resolve(&fake, &relaxed).unwrap();
        assert_eq!(
            slot(&r, "imports/mylib_v2")
                .chosen
                .as_ref()
                .unwrap()
                .revision,
            "v2.0.0"
        );
    }

    /// A depends on B and C, B on E, and B, C and E on D. B overrides D.
    fn override_graph(fake: &mut Fake, c_wants: &str) -> String {
        d_versions(fake);
        child(
            fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[
                ("libs/d", D, ", revision = \"v1.5.0\", override = true"),
                ("libs/e", E, ", revision = \"v1.0.0\""),
            ]),
        );
        child(
            fake,
            E,
            "e1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.6.0\"")]),
        );
        child(
            fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, &format!(", revision = \"{}\"", c_wants))]),
        );
        deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/c", C, ", revision = \"v1.0.0\""),
        ])
    }

    #[test]
    fn an_override_wins_over_what_it_dominates() {
        let mut fake = Fake::default();
        let config = override_graph(&mut fake, "v1.3.1");
        let r = resolve(&fake, &config).unwrap();
        let d = slot(&r, "imports/d");
        let chosen = d.chosen.as_ref().unwrap();
        assert_eq!(chosen.revision, "v1.5.0");
        assert_eq!(chosen.resolution, "override");
        let overruled: Vec<&RequestView> = d
            .requests
            .iter()
            .filter(|q| q.overruled_by.is_some())
            .collect();
        assert_eq!(overruled.len(), 1, "{:#?}", d.requests);
        assert_eq!(overruled[0].from, "imports/e");
    }

    #[test]
    fn an_override_must_agree_with_what_it_does_not_dominate() {
        let mut fake = Fake::default();
        let config = override_graph(&mut fake, "v1.6.0");
        let err = resolve(&fake, &config).unwrap_err().to_string();
        assert!(err.contains("override conflict"), "{}", err);

        // The root, which dominates everything, settles it.
        let settled = format!(
            "{}\"imports/d\" = {{ url = \"{}\", revision = \"v1.5.0\", override = true }}\n",
            config, D
        );
        let r = resolve(&fake, &settled).unwrap();
        assert_eq!(
            slot(&r, "imports/d").chosen.as_ref().unwrap().revision,
            "v1.5.0"
        );
    }

    #[test]
    fn a_branch_is_decided_by_position_not_history() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"main\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.6.0\"")]),
        );

        // The root is above everyone: its branch wins, and the branch joins
        // the checkout the versions are in.
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/c", C, ", revision = \"v1.0.0\""),
            ("imports/d", D, ", revision = \"main\""),
        ]);
        let r = resolve(&fake, &config).unwrap();
        let d = slot(&r, "imports/d");
        assert_eq!(d.majors, 1);
        let chosen = d.chosen.as_ref().unwrap();
        assert_eq!(
            (chosen.revision.as_str(), chosen.reason),
            ("main", Reason::Position)
        );
        // Without history, nothing says whether main is behind v1.6.0.
        assert!(d.requests.iter().all(|r| !r.ahead));

        // Two siblings, neither above the other: no order without history.
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/c", C, ", revision = \"v1.0.0\""),
        ]);
        let err = resolve(&fake, &config).unwrap_err().to_string();
        assert!(
            err.contains("cannot order") && err.contains("main"),
            "{}",
            err
        );
    }

    #[test]
    fn history_on_disk_only_adds_a_warning() {
        let mut fake = Fake {
            history: true,
            ..Fake::default()
        };
        d_versions(&mut fake);
        fake.repo(D).branch("stable", "d131");
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.5.0\"")]),
        );
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/c", C, ", revision = \"v1.0.0\""),
                ("imports/d", D, ", revision = \"stable\""),
            ]),
        )
        .unwrap();
        let d = slot(&r, "imports/d");
        assert_eq!(d.chosen.as_ref().unwrap().revision, "stable");
        let ahead: Vec<&str> = d
            .requests
            .iter()
            .filter(|r| r.ahead)
            .map(|r| r.from.as_str())
            .collect();
        assert_eq!(ahead, vec!["imports/c"]);
    }

    #[test]
    fn a_missing_revision_fails_unless_an_override_above_covers_it() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"develop\"")]),
        );
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/d", D, ", revision = \"v1.2.0\""),
        ]);
        let err = resolve(&fake, &config).unwrap_err().to_string();
        assert!(
            err.contains("'develop' is not a branch, tag or commit"),
            "{}",
            err
        );
        assert!(err.contains("root → imports/b@v1.0.0"), "{}", err);

        // The root is above b: its override replaces the broken pin.
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/d", D, ", revision = \"v1.2.0\", override = true"),
        ]);
        let r = resolve(&fake, &config).unwrap();
        let d = slot(&r, "imports/d");
        assert_eq!(d.chosen.as_ref().unwrap().revision, "v1.2.0");
        let broken = d.requests.iter().find(|q| q.from == "imports/b").unwrap();
        assert!(broken.missing);
        assert_eq!(broken.overruled_by.as_deref(), Some("root"));
    }

    /// Two tied overrides, neither above the other: each wins over what its
    /// own repository is above.
    #[test]
    fn tied_overrides_each_win_over_what_they_are_above() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.6.0\"")]),
        );
        // b's override sorts first; c is below e, the second one.
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.5.0\", override = true")]),
        );
        child(
            &mut fake,
            E,
            "e1",
            "v1.0.0",
            &deps(&[
                ("libs/d", D, ", revision = \"v1.5.0\", override = true"),
                ("libs/c", C, ", revision = \"v1.0.0\""),
            ]),
        );
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/e", E, ", revision = \"v1.0.0\""),
                ("imports/b", B, ", revision = \"v1.0.0\""),
            ]),
        )
        .unwrap();
        let d = slot(&r, "imports/d");
        assert_eq!(d.chosen.as_ref().unwrap().revision, "v1.5.0");
        let beaten = d.requests.iter().find(|q| q.from == "imports/c").unwrap();
        assert_eq!(beaten.overruled_by.as_deref(), Some("imports/e"));
    }

    /// The root declares one major at the plain name; a dependency's other
    /// major is placed beside it rather than on it.
    #[test]
    fn an_implicit_major_never_lands_on_a_root_checkout() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/mylib", D, ", revision = \"v1.5.0\"")]),
        );
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/b", B, ", revision = \"v1.0.0\""),
                ("imports/mylib", D, ", revision = \"v2.0.0\""),
            ]),
        )
        .unwrap();
        assert_eq!(
            slot(&r, "imports/mylib").chosen.as_ref().unwrap().revision,
            "v2.0.0"
        );
        assert_eq!(
            slot(&r, "imports/mylib_v1")
                .chosen
                .as_ref()
                .unwrap()
                .revision,
            "v1.5.0"
        );
    }

    /// Only `[repos]` and `singleton` of a dependency's config are read: what
    /// else it holds is its own business, and cannot break the parent.
    #[test]
    fn a_dependency_config_is_read_for_its_repos_only() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        let config = format!(
            "[artefact]\nroot = \"dist\"\n\n[resolve]\nhoist = \"x\"\n\n[hooks]\non_pull_error = \"maybe\"\n\n{}",
            deps(&[("libs/d", D, ", revision = \"v1.5.0\"")])
        );
        child(&mut fake, B, "b1", "v1.0.0", &config);
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/b", B, ", revision = \"v1.0.0\""),
                ("imports/d", D, ", revision = \"v1.2.0\""),
            ]),
        )
        .unwrap();
        assert_eq!(
            slot(&r, "imports/d").chosen.as_ref().unwrap().revision,
            "v1.5.0"
        );

        // Its entries are still held to the rules: they decide what is cloned.
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("../escape", D, ", revision = \"v1.5.0\"")]),
        );
        assert!(resolve(&fake, &deps(&[("imports/b", B, ", revision = \"v1.0.0\"")])).is_err());
    }

    #[test]
    fn cycles_fail() {
        let mut fake = Fake::default();
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/c", C, ", revision = \"v1.0.0\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/b", B, ", revision = \"v1.0.0\"")]),
        );
        let err = resolve(&fake, &deps(&[("imports/b", B, ", revision = \"v1.0.0\"")]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("cycle"), "{}", err);
    }

    #[test]
    fn a_losing_revision_withdraws_its_requests() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        // B v1.0.0 needs E; B v1.1.0 no longer does.
        fake.repo(B)
            .commit(
                "b10",
                None,
                Some(&deps(&[("libs/e", E, ", revision = \"v1.0.0\"")])),
            )
            .commit("b11", Some("b10"), Some("[repos]\n"))
            .tag("v1.0.0", "b10")
            .tag("v1.1.0", "b11")
            .branch("main", "b11");
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/b", B, ", revision = \"v1.1.0\"")]),
        );
        child(&mut fake, E, "e1", "v1.0.0", "[repos]\n");
        let r = resolve(
            &fake,
            &deps(&[
                ("imports/b", B, ", revision = \"v1.0.0\""),
                ("imports/c", C, ", revision = \"v1.0.0\""),
            ]),
        )
        .unwrap();
        assert_eq!(
            slot(&r, "imports/b").chosen.as_ref().unwrap().revision,
            "v1.1.0"
        );
        assert!(r.slot("imports/e").is_none(), "{:#?}", r.slots);
    }

    #[test]
    fn implicit_dependencies_need_the_allowlist() {
        let mut fake = Fake::default();
        let stranger = "https://github.com/stranger/x.git";
        fake.repo(stranger)
            .commit("x1", None, None)
            .tag("v1.0.0", "x1");
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/x", stranger, ", revision = \"v1.0.0\"")]),
        );
        let config = deps(&[("imports/b", B, ", revision = \"v1.0.0\"")]);
        let err = resolve(&fake, &config).unwrap_err().to_string();
        assert!(err.contains("github.com/stranger/*"), "{}", err);

        let allowed = format!(
            "[resolve]\nallow = [\"github.com/stranger/*\"]\n\n{}",
            config
        );
        assert!(resolve(&fake, &allowed)
            .unwrap()
            .slot("imports/x")
            .is_some());
    }

    // -----------------------------------------------------------------------
    // Topics
    // -----------------------------------------------------------------------

    /// Checkouts at given heads, some with an edited config on disk.
    #[derive(Default)]
    struct AtHeads {
        heads: BTreeMap<String, String>,
        configs: BTreeMap<String, String>,
    }

    impl Checkouts for AtHeads {
        fn config_if_at(&self, dir: &str, _: Kind, commit: &str) -> Option<Option<String>> {
            let head = self.heads.get(dir)?;
            (head == commit).then(|| self.configs.get(dir).cloned())
        }
        fn head(&self, dir: &str) -> Option<String> {
            self.heads.get(dir).cloned()
        }
    }

    fn resolve_on(
        fake: &Fake,
        checkouts: &AtHeads,
        config: &str,
        topic: &str,
    ) -> Result<Resolution> {
        let config = root(config);
        Engine::new(&config, None, fake, checkouts)
            .with_topic(Some(topic.to_string()))
            .resolve()
    }

    /// B v1.0.0 needs D v1.3.1; the root asks for B.
    fn topic_graph() -> (Fake, String) {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.3.1\"")]),
        );
        (fake, deps(&[("imports/b", B, ", revision = \"v1.0.0\"")]))
    }

    #[test]
    fn a_local_topic_branch_beats_every_request_and_keeps_the_pin() {
        let (mut fake, config) = topic_graph();
        fake.local
            .entry(D.to_string())
            .or_default()
            .insert("feat/x".into(), "dlocal".into());
        let r = resolve_on(&fake, &AtHeads::default(), &config, "feat/x").unwrap();
        let d = slot(&r, "imports/d");
        let chosen = d.chosen.as_ref().unwrap();
        assert_eq!(chosen.revision, "feat/x");
        assert_eq!(chosen.commit, "dlocal");
        assert_eq!(chosen.reason, Reason::Topic);
        assert_eq!(chosen.by, "topic");
        assert_eq!(d.commit.as_deref(), Some("dlocal"));
        let topic = d.topic.as_ref().unwrap();
        assert!(topic.developed);
        let pin = topic.pin.as_ref().unwrap();
        assert_eq!(
            (pin.revision.as_str(), pin.by.as_str()),
            ("v1.3.1", "imports/b")
        );
        assert_eq!(d.entry().revision, "feat/x");
        // The topic's own request is not one a repository made.
        assert_eq!(d.requests.len(), 1);
        // B has no branch of the topic: at its pin.
        assert!(slot(&r, "imports/b").topic.is_none());
        assert_eq!(slot(&r, "imports/b").branch.as_deref(), Some("feat/x"));
    }

    #[test]
    fn a_topic_slots_own_config_is_read_where_it_stands() {
        let (mut fake, config) = topic_graph();
        fake.repo(E).commit("e1", None, None).tag("v1.0.0", "e1");
        fake.local
            .entry(B.to_string())
            .or_default()
            .insert("feat/x".into(), "blocal".into());
        let mut checkouts = AtHeads::default();
        checkouts.heads.insert("imports/b".into(), "blocal".into());
        // B on the branch now also needs E — an edit not yet committed.
        checkouts.configs.insert(
            "imports/b".into(),
            deps(&[
                ("libs/d", D, ", revision = \"v1.3.1\""),
                ("libs/e", E, ", revision = \"v1.0.0\""),
            ]),
        );
        let r = resolve_on(&fake, &checkouts, &config, "feat/x").unwrap();
        assert!(slot(&r, "imports/b").topic.is_some());
        assert_eq!(
            slot(&r, "imports/e").chosen.as_ref().unwrap().revision,
            "v1.0.0"
        );
    }

    #[test]
    fn a_remote_branch_of_the_topics_name_is_followed_unless_the_root_overrides() {
        let (mut fake, config) = topic_graph();
        fake.repo(D)
            .commit("dfeat", Some("d131"), None)
            .branch("feat/x", "dfeat");
        let none = AtHeads::default();
        let r = resolve_on(&fake, &none, &config, "feat/x").unwrap();
        let d = slot(&r, "imports/d");
        assert_eq!(d.commit.as_deref(), Some("dfeat"));
        assert!(!d.topic.as_ref().unwrap().developed);

        let held = format!(
            "{}\"imports/d\" = {{ url = \"{}\", revision = \"v1.2.0\", override = true }}\n",
            config, D
        );
        let r = resolve_on(&fake, &none, &held, "feat/x").unwrap();
        let d = slot(&r, "imports/d");
        assert!(d.topic.is_none());
        assert!(d.branch.is_none(), "an override can not be developed");
        assert_eq!(d.chosen.as_ref().unwrap().revision, "v1.2.0");

        // Without a topic, a branch of that name is just a branch.
        let r = resolve(&fake, &config).unwrap();
        assert!(slot(&r, "imports/d").topic.is_none());
        assert!(slot(&r, "imports/d").branch.is_none());
    }

    #[test]
    fn each_major_has_a_branch_of_its_own() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        fake.repo(D)
            .branch("feat/x", "d200")
            .branch("feat/x@v1", "d150");
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.3.1\"")]),
        );
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v2.0.0\"")]),
        );
        let config = deps(&[
            ("imports/b", B, ", revision = \"v1.0.0\""),
            ("imports/c", C, ", revision = \"v1.0.0\""),
        ]);
        let r = resolve_on(&fake, &AtHeads::default(), &config, "feat/x").unwrap();
        let v1 = slot(&r, "imports/d");
        let v2 = slot(&r, "imports/d_v2");
        assert_eq!(v1.topic.as_ref().unwrap().branch, "feat/x@v1");
        assert_eq!(v1.commit.as_deref(), Some("d150"));
        assert_eq!(v2.topic.as_ref().unwrap().branch, "feat/x");
        assert_eq!(v2.commit.as_deref(), Some("d200"));
    }

    /// A requester whose own config pins the topic branch keeps what it asks
    /// for at its pins, all the way down; pinned wins over a requester that
    /// does not pin it.
    #[test]
    fn a_requester_that_pins_the_topic_keeps_its_dependencies_at_their_pins() {
        let mut fake = Fake::default();
        d_versions(&mut fake);
        fake.repo(D)
            .commit("dfeat", Some("d131"), None)
            .branch("staging", "dfeat");
        fake.repo(E)
            .commit("e1", None, None)
            .tag("v1.0.0", "e1")
            .commit("efeat", Some("e1"), None)
            .branch("staging", "efeat");
        // D's own config asks for E.
        fake.repo(D).configs.insert(
            "d131".into(),
            deps(&[("libs/e", E, ", revision = \"v1.0.0\"")]),
        );
        child(
            &mut fake,
            B,
            "b1",
            "v1.0.0",
            &format!(
                "[develop]\npinned = [\"staging\"]\n\n{}",
                deps(&[("libs/d", D, ", revision = \"v1.3.1\"")])
            ),
        );
        let config = format!(
            "[resolve]\nallow = [\"github.com/org/*\"]\n\n{}",
            deps(&[("imports/b", B, ", revision = \"v1.0.0\"")])
        );
        let r = resolve_on(&fake, &AtHeads::default(), &config, "staging").unwrap();
        let d = slot(&r, "imports/d");
        assert!(d.topic.is_none());
        assert_eq!(d.pinned_by.as_deref(), Some("imports/b"));
        assert_eq!(d.commit.as_deref(), Some("d131"));
        let e = slot(&r, "imports/e");
        assert!(e.topic.is_none(), "below D too");
        assert_eq!(e.pinned_by.as_deref(), Some("imports/b"));

        // Another requester that does not pin it: pinned still wins.
        child(
            &mut fake,
            C,
            "c1",
            "v1.0.0",
            &deps(&[("libs/d", D, ", revision = \"v1.3.1\"")]),
        );
        let both = format!(
            "{}\"imports/c\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            config, C
        );
        let r = resolve_on(&fake, &AtHeads::default(), &both, "staging").unwrap();
        assert_eq!(
            slot(&r, "imports/d").pinned_by.as_deref(),
            Some("imports/b")
        );
    }
}
