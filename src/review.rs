use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use crate::classify::{Classified, classify};
use crate::diff::{Hunk, RowKind, diff};
use crate::highlight::annotate;
use crate::lang::Lang;
use crate::project::{LayerMode, project};
use crate::store::fingerprint;
use crate::test_files::is_test_file;

/// Unchanged lines shown around each change.
pub const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl ChangeStatus {
    pub fn letter(self) -> char {
        match self {
            ChangeStatus::Added => 'A',
            ChangeStatus::Deleted => 'D',
            ChangeStatus::Modified => 'M',
            ChangeStatus::Renamed => 'R',
        }
    }
}

/// One changed file, both versions in full.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub old: Option<String>,
    pub new: Option<String>,
    pub binary: bool,
}

/// How well declutter could find the layers in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Parsed(Lang),
    /// Parsed with syntax errors; layers near an error may be missed.
    Partial(Lang),
    /// No grammar for this file type, so nothing but whole test files is hidden.
    Unsupported,
    Binary,
}

impl Detection {
    pub fn label(self) -> String {
        match self {
            Detection::Parsed(lang) => lang.name().to_string(),
            Detection::Partial(lang) => format!("{}, partial parse", lang.name()),
            Detection::Unsupported => "no grammar".to_string(),
            Detection::Binary => "binary".to_string(),
        }
    }
}

/// A layer made of spans inside a file, as opposed to the file-level test layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanLayer {
    Comments,
    Imports,
    Logging,
}

impl SpanLayer {
    pub const ALL: [SpanLayer; 3] = [SpanLayer::Comments, SpanLayer::Imports, SpanLayer::Logging];

    /// The adjective used in messages: "comment-only hunks".
    pub fn adjective(self) -> &'static str {
        match self {
            SpanLayer::Comments => "comment",
            SpanLayer::Imports => "import",
            SpanLayer::Logging => "logging",
        }
    }

    fn spans(self, classified: &Classified) -> &[Range<usize>] {
        match self {
            SpanLayer::Comments => &classified.comments,
            SpanLayer::Imports => &classified.imports,
            SpanLayer::Logging => &classified.logging,
        }
    }
}

/// The modes of the span layers: the part of `Layers` that changes a file's diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanModes {
    pub comments: LayerMode,
    pub imports: LayerMode,
    pub logging: LayerMode,
}

impl SpanModes {
    pub const SHOWN: SpanModes = SpanModes {
        comments: LayerMode::Shown,
        imports: LayerMode::Shown,
        logging: LayerMode::Shown,
    };

    pub fn mode(self, layer: SpanLayer) -> LayerMode {
        match layer {
            SpanLayer::Comments => self.comments,
            SpanLayer::Imports => self.imports,
            SpanLayer::Logging => self.logging,
        }
    }

    /// How the file is projected: any layer set to *only* wins, and shows the union of
    /// those layers; otherwise the hidden layers are cut out.
    pub fn effective(self) -> (LayerMode, Vec<SpanLayer>) {
        for mode in [LayerMode::Only, LayerMode::Hidden] {
            let layers: Vec<SpanLayer> = SpanLayer::ALL
                .into_iter()
                .filter(|&layer| self.mode(layer) == mode)
                .collect();
            if !layers.is_empty() {
                return (mode, layers);
            }
        }
        (LayerMode::Shown, Vec::new())
    }

    /// "comment", "comment/import": the layers the projection acts on.
    pub fn adjective(self) -> String {
        let (_, layers) = self.effective();
        layers
            .iter()
            .map(|layer| layer.adjective())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// Just the comment layer set; the rest shown.
impl From<LayerMode> for SpanModes {
    fn from(comments: LayerMode) -> SpanModes {
        SpanModes {
            comments,
            ..SpanModes::SHOWN
        }
    }
}

impl From<Layers> for SpanModes {
    fn from(layers: Layers) -> SpanModes {
        SpanModes {
            comments: layers.comments,
            imports: layers.imports,
            logging: layers.logging,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModeView {
    pub hunks: Vec<Hunk>,
    /// Hunks of the full diff that have no visible change in this mode.
    pub hidden_hunks: usize,
}

/// A changed file. Its diff under each choice of layers is computed on first use.
#[derive(Debug, Clone)]
pub struct FileReview {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub detection: Detection,
    /// The whole file is test code; the test layer is file-granular.
    pub is_test: bool,
    /// Identifies this exact change (path and both versions) for review marks.
    pub fingerprint: u64,
    old: String,
    new: String,
    old_layers: Classified,
    new_layers: Classified,
    views: RefCell<HashMap<SpanModes, Rc<ModeView>>>,
}

impl FileReview {
    pub fn new(change: FileChange) -> FileReview {
        let old = change.old.unwrap_or_default();
        let new = change.new.unwrap_or_default();

        let (detection, old_layers, new_layers) = if change.binary {
            (
                Detection::Binary,
                Classified::default(),
                Classified::default(),
            )
        } else {
            match Lang::from_path(&change.path)
                .and_then(|lang| Some((lang, classify(lang, &old)?, classify(lang, &new)?)))
            {
                Some((lang, old_c, new_c)) => {
                    let detection = if old_c.partial || new_c.partial {
                        Detection::Partial(lang)
                    } else {
                        Detection::Parsed(lang)
                    };
                    (detection, old_c, new_c)
                }
                None => (
                    Detection::Unsupported,
                    Classified::default(),
                    Classified::default(),
                ),
            }
        };

        let is_test = is_test_file(&change.path, if new.is_empty() { &old } else { &new });
        let fingerprint = fingerprint(&[&change.path, &old, &new]);
        FileReview {
            path: change.path,
            old_path: change.old_path,
            status: change.status,
            detection,
            is_test,
            fingerprint,
            old,
            new,
            old_layers,
            new_layers,
            views: RefCell::new(HashMap::new()),
        }
    }

    pub fn view(&self, modes: impl Into<SpanModes>) -> Rc<ModeView> {
        let modes = modes.into();
        if let Some(view) = self.views.borrow().get(&modes) {
            return Rc::clone(view);
        }
        let hunks = self.hunks_for(modes);
        let hidden_hunks = if modes == SpanModes::SHOWN {
            0
        } else {
            count_hidden(&self.view(SpanModes::SHOWN).hunks, &hunks)
        };
        let view = Rc::new(ModeView {
            hunks,
            hidden_hunks,
        });
        self.views.borrow_mut().insert(modes, Rc::clone(&view));
        view
    }

    fn hunks_for(&self, modes: SpanModes) -> Vec<Hunk> {
        let (mode, layers) = modes.effective();
        let lang = match self.detection {
            Detection::Parsed(lang) | Detection::Partial(lang) => Some(lang),
            _ => None,
        };
        let side = |src: &str, classified: &Classified| {
            let mut projection = project(src, &union(classified, &layers), mode);
            if let Some(lang) = lang {
                annotate(&mut projection, lang);
            }
            projection
        };
        let mut hunks = diff(
            &side(&self.old, &self.old_layers),
            &side(&self.new, &self.new_layers),
            CONTEXT,
        );
        if mode == LayerMode::Hidden {
            // Added or removed blank lines are layout: they mostly travel with whatever
            // was hidden, and they never change what the code does.
            for hunk in &mut hunks {
                hunk.rows
                    .retain(|row| row.kind == RowKind::Context || !row.text.trim().is_empty());
            }
            hunks.retain(|hunk| hunk.changed().next().is_some());
        }
        hunks
    }

    pub fn total_hunks(&self) -> usize {
        self.view(SpanModes::SHOWN).hunks.len()
    }

    /// Whether the file is listed at all under this test-layer mode.
    pub fn is_visible(&self, tests: LayerMode) -> bool {
        match tests {
            LayerMode::Shown => true,
            LayerMode::Hidden => !self.is_test,
            LayerMode::Only => self.is_test,
        }
    }

    /// The bracketed tag after the file name: language or detection problem, and `test`.
    pub fn tag(&self) -> String {
        let detection = self.detection.label();
        if self.is_test {
            format!("{detection}, test")
        } else {
            detection
        }
    }
}

/// The spans of `layers`, merged into one sorted, non-overlapping list.
fn union(classified: &Classified, layers: &[SpanLayer]) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = layers
        .iter()
        .flat_map(|layer| layer.spans(classified).iter().cloned())
        .collect();
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// Counts hunks of the full diff none of whose changed lines are still changed in `visible`.
fn count_hidden(full: &[Hunk], visible: &[Hunk]) -> usize {
    let key = |kind: RowKind, old: Option<usize>, new: Option<usize>| match kind {
        RowKind::Removed => old.map(|line| (false, line)),
        _ => new.map(|line| (true, line)),
    };
    let still_changed: HashSet<(bool, usize)> = visible
        .iter()
        .flat_map(Hunk::changed)
        .filter_map(|row| key(row.kind, row.old_line, row.new_line))
        .collect();
    full.iter()
        .filter(|hunk| {
            !hunk.changed().any(|row| {
                key(row.kind, row.old_line, row.new_line)
                    .is_some_and(|k| still_changed.contains(&k))
            })
        })
        .count()
}

/// What the reviewer has chosen to see of each layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layers {
    pub comments: LayerMode,
    pub tests: LayerMode,
    pub imports: LayerMode,
    pub logging: LayerMode,
}

impl Default for Layers {
    fn default() -> Layers {
        Layers {
            comments: LayerMode::Hidden,
            tests: LayerMode::Shown,
            imports: LayerMode::Shown,
            logging: LayerMode::Shown,
        }
    }
}

/// Totals across the files visible under one choice of layers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    pub total_hunks: usize,
    pub visible_hunks: usize,
    pub hidden_hunks: usize,
    /// Files with changes, none of them visible under these layers.
    pub fully_hidden_files: usize,
    /// Files left out by the test layer: test files when hidden, the rest when only.
    pub filtered_files: usize,
    pub unsupported_files: usize,
    pub partial_files: usize,
}

impl Summary {
    pub fn new(files: &[FileReview], layers: Layers) -> Summary {
        let mut summary = Summary::default();
        for file in files {
            if !file.is_visible(layers.tests) {
                summary.filtered_files += 1;
                continue;
            }
            let view = file.view(layers);
            summary.files += 1;
            summary.total_hunks += file.total_hunks();
            summary.visible_hunks += view.hunks.len();
            summary.hidden_hunks += view.hidden_hunks;
            if file.total_hunks() > 0 && view.hunks.is_empty() {
                summary.fully_hidden_files += 1;
            }
            match file.detection {
                Detection::Unsupported => summary.unsupported_files += 1,
                Detection::Partial(_) => summary.partial_files += 1,
                _ => {}
            }
        }
        summary
    }

    pub fn status_line(&self, layers: Layers) -> String {
        let states: Vec<String> = [
            ("comments", layers.comments),
            ("tests", layers.tests),
            ("imports", layers.imports),
            ("logging", layers.logging),
        ]
        .into_iter()
        .filter(|(_, mode)| *mode != LayerMode::Shown)
        .map(|(name, mode)| format!("{name}: {}", mode.label()))
        .collect();
        let mut parts = if states.is_empty() {
            vec!["all layers shown".to_string()]
        } else {
            states
        };

        let modes = SpanModes::from(layers);
        let adjective = modes.adjective();
        match modes.effective().0 {
            LayerMode::Shown => parts.push(format!(
                "{} {} in {} {}",
                self.total_hunks,
                plural(self.total_hunks, "hunk"),
                self.files,
                plural(self.files, "file")
            )),
            LayerMode::Hidden => {
                parts.push(format!(
                    "showing {} of {} hunks",
                    self.visible_hunks, self.total_hunks
                ));
                parts.push(format!(
                    "{} {adjective}-only {} hidden",
                    self.hidden_hunks,
                    plural(self.hidden_hunks, "hunk")
                ));
                if self.fully_hidden_files > 0 {
                    parts.push(format!(
                        "{} {adjective}-only {}",
                        self.fully_hidden_files,
                        plural(self.fully_hidden_files, "file")
                    ));
                }
            }
            LayerMode::Only => {
                parts.push(format!(
                    "{} {} with {adjective} changes",
                    self.visible_hunks,
                    plural(self.visible_hunks, "hunk")
                ));
                parts.push(format!(
                    "{} other {} hidden",
                    self.hidden_hunks,
                    plural(self.hidden_hunks, "hunk")
                ));
            }
        }
        match layers.tests {
            LayerMode::Shown => {}
            LayerMode::Hidden => parts.push(format!(
                "{} test {} hidden",
                self.filtered_files,
                plural(self.filtered_files, "file")
            )),
            LayerMode::Only => parts.push(format!(
                "{} non-test {} hidden",
                self.filtered_files,
                plural(self.filtered_files, "file")
            )),
        }
        if self.unsupported_files > 0 {
            parts.push(format!(
                "no grammar for {} {}",
                self.unsupported_files,
                plural(self.unsupported_files, "file")
            ));
        }
        if self.partial_files > 0 {
            parts.push(format!(
                "{} {} partially parsed",
                self.partial_files,
                plural(self.partial_files, "file")
            ));
        }
        parts.join(" · ")
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}
