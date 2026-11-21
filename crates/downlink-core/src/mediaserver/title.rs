//! Title normalisation for matching IPTV titles against a media server's library.
//!
//! IPTV names carry provider decoration (`4K: #BringBackAlice (2023)`, `EN - The Office`,
//! `|UK| Taskmaster [HD]`). Both sides go through [`title_key`], so punctuation and case never
//! decide a match; [`strip_decoration`] removes the provider prefix and a trailing year.

use std::sync::LazyLock;

use regex::Regex;

/// `4K: `, `EN - `, `|UK| `, `[NF] `: a short upper-case tag in front of the title.
static PREFIX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:[|\[(]\s*[A-Z0-9+]{1,5}\s*[|\])]\s*-?\s*|[A-Z0-9+]{1,4}\s*(?::|\s-|\|)\s+)")
        .ok()
});
/// A trailing `(2023)` / `[2023]` / ` - 2023`.
static YEAR: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\s*(?:[(\[]((?:19|20)\d{2})[)\]]|-\s*((?:19|20)\d{2}))\s*$").ok()
});
/// Trailing quality / language tags: `[HD]`, `(4K)`, `[MULTI]`.
static TAGS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[(\[](?:hd|fhd|uhd|sd|4k|8k|hdr|multi|multi-sub|sub|dub|vost|vostfr)[)\]]\s*$",
    )
    .ok()
});

/// The title without provider decoration, and the year it carried (if any).
pub fn strip_decoration(raw: &str) -> (String, Option<i64>) {
    let mut s = raw.trim().to_owned();
    // Tags and year can come in either order at the end; peel until nothing changes.
    let mut year = None;
    for _ in 0..4 {
        let before = s.len();
        if let Some(re) = TAGS.as_ref() {
            s = re.replace(&s, "").into_owned();
        }
        if let Some(re) = YEAR.as_ref()
            && let Some(c) = re.captures(&s)
        {
            if year.is_none() {
                year = c
                    .get(1)
                    .or_else(|| c.get(2))
                    .and_then(|m| m.as_str().parse().ok());
            }
            let start = c.get(0).map_or(s.len(), |m| m.start());
            s.truncate(start);
        }
        if s.len() == before {
            break;
        }
    }
    if let Some(re) = PREFIX.as_ref() {
        let stripped = re.replace(&s, "").into_owned();
        if !stripped.trim().is_empty() {
            s = stripped;
        }
    }
    (s.trim().to_owned(), year)
}

/// Comparison key: lower-case alphanumerics separated by single spaces, `&` read as `and`,
/// apostrophes dropped, accents folded for the common Latin letters.
pub fn title_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    let push = |out: &mut String, space: &mut bool, piece: &str| {
        if *space && !out.is_empty() {
            out.push(' ');
        }
        *space = false;
        out.push_str(piece);
    };
    for ch in s.chars() {
        let folded: Option<&str> = match ch {
            '&' => Some("and"),
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                Some("a")
            }
            'ç' | 'Ç' => Some("c"),
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => Some("e"),
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => Some("i"),
            'ñ' | 'Ñ' => Some("n"),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
                Some("o")
            }
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => Some("u"),
            'ý' | 'ÿ' | 'Ý' => Some("y"),
            _ => None,
        };
        if let Some(f) = folded {
            push(&mut out, &mut space, f);
        } else if matches!(ch, '\'' | '’' | '`') {
            // "Schitt's" == "Schitts"
        } else if ch.is_alphanumeric() {
            let lower: String = ch.to_lowercase().collect();
            push(&mut out, &mut space, &lower);
        } else {
            space = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_is_stripped() {
        assert_eq!(
            strip_decoration("4K: #BringBackAlice (2023)"),
            ("#BringBackAlice".to_owned(), Some(2023))
        );
        assert_eq!(
            strip_decoration("EN - The Office"),
            ("The Office".to_owned(), None)
        );
        assert_eq!(
            strip_decoration("|UK| Taskmaster [HD]"),
            ("Taskmaster".to_owned(), None)
        );
        assert_eq!(
            strip_decoration("Heat (1995) [4K]"),
            ("Heat".to_owned(), Some(1995))
        );
        assert_eq!(
            strip_decoration("Blade Runner 2049"),
            ("Blade Runner 2049".to_owned(), None)
        );
    }

    #[test]
    fn real_colons_survive() {
        assert_eq!(
            strip_decoration("Alien: Romulus (2024)"),
            ("Alien: Romulus".to_owned(), Some(2024))
        );
        assert_eq!(strip_decoration("M3GAN: 2.0").0, "M3GAN: 2.0");
    }

    #[test]
    fn keys_ignore_punctuation_and_case() {
        assert_eq!(title_key("Star Wars: Episode IV"), "star wars episode iv");
        assert_eq!(title_key("star wars - episode iv"), "star wars episode iv");
        assert_eq!(title_key("Law & Order"), "law and order");
        assert_eq!(title_key("Schitt's Creek"), "schitts creek");
        assert_eq!(title_key("Amélie"), "amelie");
        assert_eq!(title_key("#BringBackAlice"), "bringbackalice");
    }
}
