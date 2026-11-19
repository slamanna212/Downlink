//! Quote-aware attribute scanner. Never fails: malformed input yields fewer attributes.

/// Ordered attribute list; keys are matched case-insensitively. Unknown attributes are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs(Vec<(String, String)>);

impl Attrs {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

/// Parse `key="value" key2=value2 key3='v'`. A quoted value runs to the next matching quote, or to
/// the end of input when the quote is never closed.
pub fn parse_attrs(s: &str) -> Attrs {
    let mut out = Vec::new();
    let b: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < b.len() {
        // find the start of a key
        while i < b.len() && !is_key_char(b[i].1) {
            i += 1;
        }
        let ks = i;
        while i < b.len() && is_key_char(b[i].1) {
            i += 1;
        }
        if ks == i {
            break;
        }
        let key = &s[b[ks].0..b.get(i).map_or(s.len(), |x| x.0)];
        // optional spaces then '='
        let mut j = i;
        while j < b.len() && b[j].1 == ' ' {
            j += 1;
        }
        if j >= b.len() || b[j].1 != '=' {
            continue; // a bare word, not an attribute
        }
        j += 1;
        while j < b.len() && b[j].1 == ' ' {
            j += 1;
        }
        if j >= b.len() {
            out.push((key.to_owned(), String::new()));
            break;
        }
        let q = b[j].1;
        if q == '"' || q == '\'' {
            let vs = j + 1;
            let mut e = vs;
            while e < b.len() && b[e].1 != q {
                e += 1;
            }
            let end = b.get(e).map_or(s.len(), |x| x.0);
            out.push((
                key.to_owned(),
                s[b[vs.min(b.len() - 1)].0.min(end)..end].to_owned(),
            ));
            i = (e + 1).min(b.len());
        } else {
            let vs = j;
            let mut e = vs;
            while e < b.len() && !b[e].1.is_whitespace() {
                e += 1;
            }
            let end = b.get(e).map_or(s.len(), |x| x.0);
            out.push((key.to_owned(), s[b[vs].0..end].to_owned()));
            i = e;
        }
    }
    Attrs(out)
}
