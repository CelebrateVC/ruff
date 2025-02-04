use std::borrow::Cow;
use std::collections::BTreeSet;

use itertools::Itertools;
use ropey::RopeBuilder;
use rustpython_parser::ast::Location;

use crate::ast::types::Range;
use crate::autofix::{Fix, Patch};
use crate::checks::Check;
use crate::source_code_locator::SourceCodeLocator;

#[derive(Hash)]
pub enum Mode {
    Generate,
    Apply,
    None,
}

impl From<bool> for Mode {
    fn from(value: bool) -> Self {
        if value {
            Mode::Apply
        } else {
            Mode::None
        }
    }
}

impl From<&Mode> for bool {
    fn from(value: &Mode) -> Self {
        match value {
            Mode::Generate | Mode::Apply => true,
            Mode::None => false,
        }
    }
}

/// Auto-fix errors in a file, and write the fixed source code to disk.
pub fn fix_file<'a>(
    checks: &'a [Check],
    locator: &'a SourceCodeLocator<'a>,
) -> Option<(Cow<'a, str>, usize)> {
    if checks.iter().all(|check| check.fix.is_none()) {
        return None;
    }

    Some(apply_fixes(
        checks.iter().filter_map(|check| check.fix.as_ref()),
        locator,
    ))
}

/// Apply a series of fixes.
fn apply_fixes<'a>(
    fixes: impl Iterator<Item = &'a Fix>,
    locator: &'a SourceCodeLocator<'a>,
) -> (Cow<'a, str>, usize) {
    let mut output = RopeBuilder::new();
    let mut last_pos: Location = Location::new(1, 0);
    let mut applied: BTreeSet<&Patch> = BTreeSet::default();
    let mut num_fixed: usize = 0;

    for fix in fixes.sorted_by_key(|fix| fix.patch.location) {
        // If we already applied an identical fix as part of another correction, skip
        // any re-application.
        if applied.contains(&fix.patch) {
            num_fixed += 1;
            continue;
        }

        // Best-effort approach: if this fix overlaps with a fix we've already applied,
        // skip it.
        if last_pos > fix.patch.location {
            continue;
        }

        // Add all contents from `last_pos` to `fix.patch.location`.
        let slice = locator.slice_source_code_range(&Range {
            location: last_pos,
            end_location: fix.patch.location,
        });
        output.append(&slice);

        // Add the patch itself.
        output.append(&fix.patch.content);

        // Track that the fix was applied.
        last_pos = fix.patch.end_location;
        applied.insert(&fix.patch);
        num_fixed += 1;
    }

    // Add the remaining content.
    let slice = locator.slice_source_code_at(last_pos);
    output.append(&slice);

    (Cow::from(output.finish()), num_fixed)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rustpython_parser::ast::Location;

    use crate::autofix::fixer::apply_fixes;
    use crate::autofix::{Fix, Patch};
    use crate::SourceCodeLocator;

    #[test]
    fn empty_file() -> Result<()> {
        let fixes = vec![];
        let locator = SourceCodeLocator::new(r#""#);
        let (contents, fixed) = apply_fixes(fixes.iter(), &locator);
        assert_eq!(contents, "");
        assert_eq!(fixed, 0);

        Ok(())
    }

    #[test]
    fn apply_single_replacement() -> Result<()> {
        let fixes = vec![Fix {
            patch: Patch {
                content: "Bar".to_string(),
                location: Location::new(1, 8),
                end_location: Location::new(1, 14),
            },
        }];
        let locator = SourceCodeLocator::new(
            r#"
class A(object):
    ...
"#
            .trim(),
        );
        let (contents, fixed) = apply_fixes(fixes.iter(), &locator);
        assert_eq!(
            contents,
            r#"
class A(Bar):
    ...
"#
            .trim(),
        );
        assert_eq!(fixed, 1);

        Ok(())
    }

    #[test]
    fn apply_single_removal() -> Result<()> {
        let fixes = vec![Fix {
            patch: Patch {
                content: String::new(),
                location: Location::new(1, 7),
                end_location: Location::new(1, 15),
            },
        }];
        let locator = SourceCodeLocator::new(
            r#"
class A(object):
    ...
"#
            .trim(),
        );
        let (contents, fixed) = apply_fixes(fixes.iter(), &locator);
        assert_eq!(
            contents,
            r#"
class A:
    ...
"#
            .trim()
        );
        assert_eq!(fixed, 1);

        Ok(())
    }

    #[test]
    fn apply_double_removal() -> Result<()> {
        let fixes = vec![
            Fix {
                patch: Patch {
                    content: String::new(),
                    location: Location::new(1, 7),
                    end_location: Location::new(1, 16),
                },
            },
            Fix {
                patch: Patch {
                    content: String::new(),
                    location: Location::new(1, 16),
                    end_location: Location::new(1, 23),
                },
            },
        ];
        let locator = SourceCodeLocator::new(
            r#"
class A(object, object):
    ...
"#
            .trim(),
        );
        let (contents, fixed) = apply_fixes(fixes.iter(), &locator);

        assert_eq!(
            contents,
            r#"
class A:
    ...
"#
            .trim()
        );
        assert_eq!(fixed, 2);

        Ok(())
    }

    #[test]
    fn ignore_overlapping_fixes() -> Result<()> {
        let fixes = vec![
            Fix {
                patch: Patch {
                    content: String::new(),
                    location: Location::new(1, 7),
                    end_location: Location::new(1, 15),
                },
            },
            Fix {
                patch: Patch {
                    content: "ignored".to_string(),
                    location: Location::new(1, 9),
                    end_location: Location::new(1, 11),
                },
            },
        ];
        let locator = SourceCodeLocator::new(
            r#"
class A(object):
    ...
"#
            .trim(),
        );
        let (contents, fixed) = apply_fixes(fixes.iter(), &locator);
        assert_eq!(
            contents,
            r#"
class A:
    ...
"#
            .trim(),
        );
        assert_eq!(fixed, 1);

        Ok(())
    }
}
