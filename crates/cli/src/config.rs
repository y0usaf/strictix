use std::path::{Path, PathBuf};

use strictix_core::config::LintConfig;

pub fn load_config(
    config_path: Option<&Path>,
    flags: &[String],
    schema_flag: Option<&Path>,
) -> Result<LintConfig, String> {
    let mut config = match config_path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read config file {}: {e}", path.display()))?;
            parse_config(&text)
                .map_err(|e| format!("cannot parse config file {}: {e}", path.display()))?
        }
        None => LintConfig::default(),
    };

    for code in flags {
        if !config.disabled.contains(code) {
            config.disabled.push(code.clone());
        }
    }

    config.schema = schema_flag.map(PathBuf::from).or(config.schema);

    Ok(config)
}

fn parse_config(text: &str) -> Result<LintConfig, String> {
    let mut config = LintConfig::default();
    let mut section = String::new();

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_owned();
            continue;
        }
        let rule = section
            .strip_prefix("lint.")
            .map(|code| code.trim_matches('"').to_owned());
        if section != "lint" && section != "schemas" && rule.is_none() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected `key = value`", line_no + 1));
        };
        let key = key.trim();
        let value = value.trim();
        if let Some(code) = rule {
            let err = |e: String| format!("line {}: {e}", line_no + 1);
            let values = if value.starts_with('[') {
                parse_string_array(value).map_err(err)?
            } else if value.starts_with('"') {
                vec![parse_toml_string(value).map_err(err)?]
            } else {
                vec![value.to_owned()]
            };
            config
                .settings
                .retain(|(c, k, _)| !(*c == code && k == key));
            for setting in values {
                config
                    .settings
                    .push((code.clone(), key.to_owned(), setting));
            }
            continue;
        }
        if section == "schemas" {
            let err = |e: String| format!("line {}: {e}", line_no + 1);
            let prefix = if key.starts_with('"') {
                parse_toml_string(key).map_err(err)?
            } else {
                key.to_owned()
            };
            let schema = parse_toml_string(value).map_err(err)?;
            config.schemas.push((
                PathBuf::from(prefix),
                (!schema.is_empty()).then(|| PathBuf::from(schema)),
            ));
            continue;
        }
        match key {
            "disabled" => {
                config.disabled =
                    parse_string_array(value).map_err(|e| format!("line {}: {e}", line_no + 1))?;
            }
            "enabled" => {
                config.enabled =
                    parse_string_array(value).map_err(|e| format!("line {}: {e}", line_no + 1))?;
            }
            "generated" => {
                config.generated = parse_string_array(value)
                    .map_err(|e| format!("line {}: {e}", line_no + 1))?
                    .into_iter()
                    .map(PathBuf::from)
                    .collect();
            }
            "schema" => {
                config.schema = Some(PathBuf::from(
                    parse_toml_string(value).map_err(|e| format!("line {}: {e}", line_no + 1))?,
                ));
            }
            _ => {}
        }
    }

    Ok(config)
}

fn parse_string_array(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err("expected an array of strings".to_owned());
    }
    let mut rest = value[1..value.len() - 1].trim();
    let mut items = Vec::new();
    while !rest.is_empty() {
        if !rest.starts_with('"') {
            return Err("expected a quoted string".to_owned());
        }
        let mut chars = rest.char_indices();
        chars.next();
        let mut escaped = false;
        let mut end = None;
        for (idx, c) in chars {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                end = Some(idx);
                break;
            }
        }
        let end = end.ok_or_else(|| "unterminated quoted string".to_owned())?;
        items.push(parse_toml_string(&rest[..=end])?);
        rest = rest[end + 1..].trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
        } else {
            return Err("expected ',' between array elements".to_owned());
        }
    }
    Ok(items)
}

fn parse_toml_string(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err("expected a quoted string".to_owned());
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{0008}'),
                Some('f') => out.push('\u{000C}'),
                Some(other) => return Err(format!("invalid escape '\\{other}'")),
                None => return Err("unterminated escape".to_owned()),
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}
