//! Options-schema rule: flags NixOS option names that are not declared
//! in options.json (M8).
//!
//! NixOS modules consume their module arguments through the `config`
//! formal: `config.services.example.enable` reads an option. AI-generated
//! modules frequently hallucinate option names (`config.services.exmaple.enable`)
//! that exist nowhere in nixpkgs. This rule loads the options schema
//! (shape `{"options": {"a.b.c": {...}}}`) once per process, then walks
//! every select chain rooted at a `config` module argument and flags
//! full paths absent from the schema.
//!
//! The parsed schema lives in a process-wide [OnceLock] rather than a
//! field: rules are constructed by the registry macro as unit structs
//! (`Type {}`), so the struct itself carries no state. [OnceLock] is
//! Send + Sync, keeping the rule shareable across worker threads.
//!
//! Across runs, the parsed path-set is cached on disk next to a content
//! fingerprint of options.json (below), so a second invocation re-reads
//! a few KB instead of re-parsing a multi-MB schema. The cache lives in
//! the OS temp dir and never touches the schema file's directory, so
//! linting a config repo leaves no stray files in it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::json::JsonValue;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, Reference, SemanticModel};
use strictix_syntax::{AstNode, AttrName, Expr, SelectExpr};

/// The parsed options schema: the set of declared option paths, or the
/// reason loading/parsing failed. Loaded once per process.
type Schema = Result<HashSet<String>, String>;

static OPTIONS_SCHEMA: OnceLock<Schema> = OnceLock::new();

/// FNV-1a 64-bit hash of `bytes` — a dependency-free content fingerprint
/// so the disk cache can detect that options.json changed.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Cache file name for a schema: `{content-hash:x}.paths`. Two schema
/// files are distinguished by their content, not their path, so moving
/// a config repo never invalidates the cache.
fn cache_file_for(schema: &Path) -> PathBuf {
    let hash = std::fs::read(schema)
        .map(|bytes| fnv1a(&bytes))
        .unwrap_or(0); // unreadable => hash 0, which matches a miss
    std::env::temp_dir()
        .join("strictix-cache")
        .join(format!("{hash:016x}.paths"))
}

/// Serialize the path-set as a caching-friendly form: the fingerprint in
/// braces on line 1, then one option path per line. Paths are plain
/// ASCII (ident-dot-ident), so a line-per-path format round-trips.
fn write_cache(cache: &Path, hash: u64, options: &HashSet<String>) {
    let Some(parent) = cache.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let mut out = String::with_capacity(options.len() * 24);
    out.push_str(&format!("{{{hash:016x}}}\n"));
    for key in options {
        out.push_str(key);
        out.push('\n');
    }
    let _ = std::fs::write(cache, out);
}

/// Read the on-disk cache, returning the path-set if it is present and
/// its stored fingerprint matches `expected` (i.e. options.json is
/// unchanged since the cache was written).
fn read_cache(cache: &Path, expected: u64) -> Option<HashSet<String>> {
    let text = std::fs::read_to_string(cache).ok()?;
    let mut lines = text.lines();
    let header = lines.next()?;
    if header != format!("{{{expected:016x}}}") {
        return None; // stale: options.json changed since we cached
    }
    Some(lines.map(ToOwned::to_owned).collect())
}

/// Read and parse options.json, extracting the `options` keys.
///
/// This is the authoritative path; callers use it only on a cache miss.
fn parse_schema(path: &Path) -> Schema {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let parsed = JsonValue::parse(&text).map_err(|e| e.message)?;
    let options = parsed
        .get("options")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "expected a top-level \"options\" object".to_string())?;
    Ok(options.iter().map(|(key, _)| key.clone()).collect())
}

/// Load (and cache) options.json, consulting the disk cache first.
///
/// Best-effort: a cache that cannot be read or written falls back to
/// parsing the schema directly, so a lock-free unwritable temp dir never
/// degrades correctness — only speed.
fn load_schema(path: &Path) -> Schema {
    let cache = cache_file_for(path);
    let hash = std::fs::read(path).map(|b| fnv1a(&b)).unwrap_or(0);
    if let Some(options) = read_cache(&cache, hash) {
        return Ok(options);
    }
    let options = parse_schema(path)?;
    write_cache(&cache, hash, &options);
    Ok(options)
}

/// Whether a reference is a use of the `config` module argument: a
/// LambdaParam binding named `config`.
fn is_config_reference(model: &SemanticModel, reference: &Reference) -> bool {
    reference.name.text(model.source()) == "config"
        && model
            .resolve(reference.name)
            .is_some_and(|b| b.kind == BindingKind::LambdaParam)
}

/// The select chain rooted at `ref_range`: the innermost SelectExpr
/// whose base is the referenced ident, then every SelectExpr whose base
/// is the previous one (nested chains such as `Select(Select(config,a),b)`).
/// The parser usually flattens a path into one SelectExpr with a
/// multi-element attrpath, so the chain is typically a single node.
fn select_chain<'a>(
    model: &SemanticModel<'a>,
    ref_range: strictix_syntax::TextRange,
) -> Vec<SelectExpr<'a>> {
    let root = model.root();
    let mut chain = Vec::new();
    let Some(innermost) = root
        .descendants()
        .filter_map(SelectExpr::cast)
        .find(|s| matches!(s.base(), Some(Expr::Ident(t)) if t.range() == ref_range))
    else {
        return chain;
    };
    chain.push(innermost);
    let mut current_range = innermost.range();
    loop {
        let next = root
            .descendants()
            .filter_map(SelectExpr::cast)
            .find(|s| s.base().is_some_and(|b| b.range() == current_range));
        match next {
            Some(select) => {
                current_range = select.range();
                chain.push(select);
            }
            None => break,
        }
    }
    chain
}

/// Flags attribute paths read off `config` that options.json does not
/// declare.
///
/// Only fires when [LintConfig::schema] is set; the schema is loaded
/// lazily once. A load/parse failure is reported as a single diagnostic
/// on the first `config` reference, so a broken schema cannot crash
/// the run.
pub struct UnknownOption;

impl Rule for UnknownOption {
    fn code(&self) -> &'static str {
        "unknown-option"
    }

    fn name(&self) -> &'static str {
        "Unknown option"
    }

    fn description(&self) -> &'static str {
        "Flags option paths read off the config module argument that are not declared in options.json. Hallucinated option names are a classic failure mode of AI-generated NixOS modules."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(path) = &config.schema else {
            return; // schema rule off for this run
        };
        let schema = OPTIONS_SCHEMA.get_or_init(|| load_schema(path));
        let options = match schema {
            Ok(options) => options,
            Err(err) => {
                if let Some(first) = model
                    .references()
                    .iter()
                    .find(|r| is_config_reference(model, r))
                {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("could not load options schema: {err}"),
                        first.name.range(),
                    ));
                }
                return;
            }
        };
        for reference in model.references() {
            if !is_config_reference(model, reference) {
                continue;
            }
            let chain = select_chain(model, reference.name.range());
            if chain.is_empty() {
                continue;
            }
            // Collect the path segments innermost-out, skipping chains
            // with any non-ident segment (strings, interpolations).
            let mut segments = Vec::new();
            let mut valid = true;
            for select in &chain {
                let Some(attrpath) = select.attrpath() else {
                    valid = false;
                    break;
                };
                for element in attrpath.elements() {
                    match element {
                        AttrName::Ident(token) => {
                            segments.push(token.text(model.source()).to_string())
                        }
                        AttrName::Str(_) | AttrName::Interp(_) => valid = false,
                    }
                }
            }
            if !valid {
                continue;
            }
            let full_path = segments.join(".");
            if options.contains(&full_path) {
                continue;
            }
            let range = chain
                .last()
                .and_then(|s| s.attrpath())
                .map(|a| a.range())
                .unwrap_or_else(|| chain.last().expect("chain is non-empty").range());
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("option '{full_path}' is not declared in options.json"),
                range,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A throwaway options.json in a unique temp file (deleted after).
    fn temp_schema(text: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!("strictix-test-schema-{}-{n}", std::process::id()));
        let mut f = std::fs::File::create(&path).expect("create temp schema");
        f.write_all(text.as_bytes()).expect("write temp schema");
        path
    }

    #[test]
    fn load_schema_parses_and_caches() {
        let path =
            temp_schema(r#"{"options": {"a.b": {}, "c.d": {}, "environment.systemPackages": {}}}"#);
        let schema = load_schema(&path).expect("loads and parses");
        assert!(schema.contains("a.b"));
        assert!(schema.contains("environment.systemPackages"));
        assert!(!schema.contains("a.c"));
        // A cache file now exists and read_cache returns the same set.
        let cache = cache_file_for(&path);
        assert!(cache.exists(), "cache file written on successful load");
        let hash = fnv1a(std::fs::read(&path).unwrap().as_slice());
        assert_eq!(read_cache(&cache, hash).expect("cache readable"), schema);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn cache_is_stale_after_schema_change() {
        let path = temp_schema(r#"{"options": {"a.b": {}}}"#);
        let schema = load_schema(&path).expect("first load");
        assert!(schema.contains("a.b"));
        let cache = cache_file_for(&path);
        let hash = fnv1a(std::fs::read(&path).unwrap().as_slice());
        // Now write a different schema to the same path and bump its
        // content: the stored fingerprint no longer matches, so
        // read_cache must report a miss.
        std::fs::write(&path, r#"{"options": {"new.opt": {}}}"#).expect("rewrite schema");
        let reloaded = load_schema(&path).expect("reloads despite stale cache");
        assert!(reloaded.contains("new.opt"));
        assert!(!reloaded.contains("a.b"));
        let hash_after = fnv1a(std::fs::read(&path).unwrap().as_slice());
        assert_ne!(hash, hash_after, "content hash changed");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn missing_schema_is_error_not_cache_hit() {
        let missing = std::env::temp_dir().join("strictix-no-such-schema-anywhere");
        // The cache fingerprint for an unreadable path is 0; this must not
        // accidentally return an empty cache as a hit.
        let result = load_schema(&missing);
        assert!(result.is_err(), "missing schema is an error");
    }
}
