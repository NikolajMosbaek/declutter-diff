use similar::{Algorithm, DiffTag, capture_diff_slices, group_diff_ops};

use crate::project::Projection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    /// One-based line number in the original old file.
    pub old_line: Option<usize>,
    /// One-based line number in the original new file.
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub rows: Vec<Row>,
}

impl Hunk {
    pub fn changed(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| row.kind != RowKind::Context)
    }

    pub fn header(&self) -> String {
        let old = self.rows.iter().find_map(|row| row.old_line);
        let new = self.rows.iter().find_map(|row| row.new_line);
        let fmt = |line: Option<usize>| line.map_or("0".to_string(), |n| n.to_string());
        format!("@@ -{} +{} @@", fmt(old), fmt(new))
    }
}

/// Diffs two projections line by line and groups the result into hunks with
/// `context` unchanged lines around each change.
pub fn diff(old: &Projection, new: &Projection, context: usize) -> Vec<Hunk> {
    let ops = capture_diff_slices(Algorithm::Histogram, &old.lines, &new.lines);
    let old_row = |i: usize, kind| Row {
        kind,
        old_line: Some(old.orig[i] + 1),
        new_line: None,
        text: old.lines[i].clone(),
    };
    let new_row = |j: usize| Row {
        kind: RowKind::Added,
        old_line: None,
        new_line: Some(new.orig[j] + 1),
        text: new.lines[j].clone(),
    };

    group_diff_ops(ops, context)
        .into_iter()
        .map(|group| {
            let mut rows = Vec::new();
            for op in group {
                let (tag, old_range, new_range) = op.as_tag_tuple();
                match tag {
                    DiffTag::Equal => {
                        for (i, j) in old_range.zip(new_range) {
                            rows.push(Row {
                                new_line: Some(new.orig[j] + 1),
                                ..old_row(i, RowKind::Context)
                            });
                        }
                    }
                    DiffTag::Delete => rows.extend(old_range.map(|i| old_row(i, RowKind::Removed))),
                    DiffTag::Insert => rows.extend(new_range.map(new_row)),
                    DiffTag::Replace => {
                        rows.extend(old_range.map(|i| old_row(i, RowKind::Removed)));
                        rows.extend(new_range.map(new_row));
                    }
                }
            }
            Hunk { rows }
        })
        .collect()
}
