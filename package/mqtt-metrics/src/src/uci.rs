// SPDX-License-Identifier: GPL-2.0-or-later
//! Parser for `uci -q show <config>` output.
//!
//! `uci show` prints one line per section (`cfg.name=type`) and per option
//! (`cfg.name.option='value'`, lists as `'a' 'b'`). A single-element list is
//! indistinguishable from an option, so every value is kept as a list and the
//! accessors decide how to read it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub stype: String,
    pub options: Vec<(String, Vec<String>)>,
}

impl Section {
    pub fn get(&self, option: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(name, _)| name == option)
            .and_then(|(_, values)| values.first())
            .map(String::as_str)
    }

    pub fn list(&self, option: &str) -> Vec<String> {
        self.options
            .iter()
            .find(|(name, _)| name == option)
            .map(|(_, values)| values.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    pub sections: Vec<Section>,
}

impl Package {
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn sections_of_type<'a>(
        &'a self,
        stype: &'a str,
    ) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections.iter().filter(move |s| s.stype == stype)
    }
}

/// Parse `uci show` output. Lines that do not belong to `config` or cannot be
/// parsed are skipped; `uci` itself guarantees the format.
pub fn parse(config: &str, text: &str) -> Package {
    let mut package = Package::default();
    let prefix = format!("{config}.");
    for line in text.lines() {
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let Some((path, raw_value)) = rest.split_once('=') else {
            continue;
        };
        match path.split_once('.') {
            None => {
                package.sections.push(Section {
                    name: path.to_string(),
                    stype: raw_value.to_string(),
                    options: Vec::new(),
                });
            }
            Some((section, option)) => {
                let values = parse_values(raw_value);
                if let Some(target) = package
                    .sections
                    .iter_mut()
                    .rev()
                    .find(|s| s.name == section)
                {
                    target.options.push((option.to_string(), values));
                }
            }
        }
    }
    package
}

/// Parse `'a' 'b\''c'` style value lists. Unquoted words are accepted too.
fn parse_values(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut have_token = false;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quote {
            if c == '\'' {
                in_quote = false;
            } else {
                current.push(c);
            }
        } else if c == '\'' {
            in_quote = true;
            have_token = true;
        } else if c == '\\' && i + 1 < chars.len() {
            // `'\''` is how uci escapes a quote inside a quoted value.
            current.push(chars[i + 1]);
            have_token = true;
            i += 1;
        } else if c == ' ' {
            if have_token {
                values.push(std::mem::take(&mut current));
                have_token = false;
            }
        } else {
            current.push(c);
            have_token = true;
        }
        i += 1;
    }
    if have_token {
        values.push(current);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_options_and_lists() {
        let text = "\
network.wan=interface
network.wan.proto='pppoe'
network.wan.device='eth2'
network.lan=interface
network.lan.ip6addr='fd00::1/64' 'fd01::1/64'
network.@route[0]=route
network.@route[0].target='0.0.0.0'
other.x=y
";
        let pkg = parse("network", text);
        assert_eq!(pkg.sections.len(), 3);
        let wan = pkg.section("wan").unwrap();
        assert_eq!(wan.stype, "interface");
        assert_eq!(wan.get("proto"), Some("pppoe"));
        assert_eq!(
            pkg.section("lan").unwrap().list("ip6addr"),
            vec!["fd00::1/64", "fd01::1/64"]
        );
        assert_eq!(
            pkg.section("@route[0]").unwrap().get("target"),
            Some("0.0.0.0")
        );
        assert_eq!(pkg.sections_of_type("interface").count(), 2);
    }

    #[test]
    fn handles_escaped_quotes_spaces_and_empty_values() {
        assert_eq!(parse_values("'it'\\''s a test'"), vec!["it's a test"]);
        assert_eq!(parse_values("''"), vec![""]);
        assert_eq!(parse_values("plain"), vec!["plain"]);
        assert_eq!(parse_values("'a' 'b c'"), vec!["a", "b c"]);
    }
}
