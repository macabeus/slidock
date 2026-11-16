//! Weight names. Presentations often name a weight inside the family (`Roboto Medium`, `Oswald SemiBold`) because
//! the authoring app listed each weight as its own family; a request like that binds to the family with the named
//! weight.

use slidock_canon::{is_js_whitespace, js_trim};

const WEIGHT_NAMES: &[(&[&str], u32)] = &[
    (&["thin", "hairline"], 100),
    (&["extralight", "ultralight"], 200),
    (&["light", "semilight"], 300),
    (&["regular", "normal", "book"], 400),
    (&["medium"], 500),
    (&["semibold", "demibold", "demi"], 600),
    (&["bold"], 700),
    (&["extrabold", "ultrabold"], 800),
    (&["black", "heavy", "extrablack", "ultrablack"], 900),
];

/// The numeric weight of a weight name (`SemiBold`, `Extra Light`, `demi-bold`), if it is one.
pub fn weight_of_name(name: &str) -> Option<u32> {
    let key: String = name
        .chars()
        .filter(|&c| !is_js_whitespace(c) && c != '_' && c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    WEIGHT_NAMES
        .iter()
        .find(|(names, _)| names.contains(&key.as_str()))
        .map(|&(_, weight)| weight)
}

/// A family name with a trailing weight name split off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightedFamily<'a> {
    pub family: &'a str,
    pub weight: u32,
}

/// The byte ranges of the runs of whitespace and hyphens in `name`.
fn separators(name: &str) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (at, c) in name.char_indices() {
        if !(is_js_whitespace(c) || c == '-') {
            continue;
        }
        let end = at + c.len_utf8();
        match runs.last_mut() {
            Some(run) if run.1 == at => run.1 = end,
            _ => runs.push((at, end)),
        }
    }
    runs
}

/// Splits a trailing weight name off a requested family: `Roboto Medium` → Roboto, 500. The longest matching
/// suffix of one or two words wins, so `Inter Extra Light` is Inter at 200, not `Inter Extra` at 300.
pub fn split_weight_name(family: &str) -> Option<WeightedFamily<'_>> {
    let name = js_trim(family);
    let runs = separators(name);
    [2, 1].into_iter().find_map(|take| {
        let &(start, end) = runs.get(runs.len().checked_sub(take)?)?;
        if start == 0 {
            return None;
        }
        let weight = weight_of_name(name.get(end..)?)?;
        Some(WeightedFamily {
            family: name.get(..start)?,
            weight,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(family: &str) -> Option<(&str, u32)> {
        split_weight_name(family).map(|split| (split.family, split.weight))
    }

    #[test]
    fn parses_spellings_of_weight_names() {
        assert_eq!(weight_of_name("SemiBold"), Some(600));
        assert_eq!(weight_of_name("Extra Light"), Some(200));
        assert_eq!(weight_of_name("demi-bold"), Some(600));
        assert_eq!(weight_of_name("Ultra_Black"), Some(900));
        assert_eq!(weight_of_name("Italic"), None);
        assert_eq!(weight_of_name(""), None);
    }

    #[test]
    fn splits_a_trailing_weight_name_off_a_family_longest_suffix_first() {
        assert_eq!(split("Roboto Medium"), Some(("Roboto", 500)));
        assert_eq!(split("Oswald Medium"), Some(("Oswald", 500)));
        assert_eq!(split("Inter Extra Light"), Some(("Inter", 200)));
        assert_eq!(split("Open Sans-SemiBold"), Some(("Open Sans", 600)));
        assert_eq!(split("Black"), None);
        assert_eq!(split("Roboto"), None);
    }

    #[test]
    fn trims_the_name_and_treats_a_run_of_separators_as_one() {
        assert_eq!(split("  Roboto \u{a0}- Medium  "), Some(("Roboto", 500)));
        assert_eq!(split("-Medium"), None);
        assert_eq!(split("Roboto Medium Italic"), None);
    }
}
