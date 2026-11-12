//! Static application metadata for the About window.
//!
//! Version, description and licence come from Cargo's own `CARGO_PKG_*` variables, while the
//! compiler version and build target are published by `build.rs` as `EPUB_READER_*` env vars (a
//! library crate cannot query the toolchain at runtime). The dependency list is parsed from the
//! manifest embedded at compile time with `include_str!`, so it always describes the exact
//! manifest this binary was built from. Parsing is hand-rolled — the About data needs one table
//! of a manifest it already owns — rather than adding a TOML parser dependency for a static view.

use std::sync::LazyLock;

pub const PRODUCT_NAME: &str = "Another Epub Reader";
/// Cargo package version, i.e. the value of `[package].version` at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// `rustc` version that compiled this binary, published by `build.rs` (`"unknown"` if unavailable).
// `Option::unwrap_or` is not a stable const fn, so the fallback is spelled out with `match`.
pub const RUST_VERSION: &str = match option_env!("EPUB_READER_RUSTC_VERSION") {
    Some(version) => version,
    None => "unknown",
};
/// Target triple this binary was compiled for, published by `build.rs`.
pub const BUILD_TARGET: &str = match option_env!("EPUB_READER_TARGET") {
    Some(target) => target,
    None => "unknown",
};
pub const BUILD_PROFILE: &str = if cfg!(debug_assertions) { "debug" } else { "release" };

/// One direct runtime dependency, as displayed by the About window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// Version requirement from the manifest; `"unknown"` for path/git dependencies that carry no
    /// version of their own.
    pub version: String,
}

/// Everything the About window shows about this build.
pub struct AboutInfo {
    pub product_name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub license: &'static str,
    pub rust_version: &'static str,
    pub target: &'static str,
    pub profile: &'static str,
    /// Version of `gpui` from the manifest, when it is a direct dependency.
    pub gpui_version: Option<String>,
    /// Direct runtime dependencies, in manifest order.
    pub packages: Vec<Package>,
}

/// The manifest this build was compiled from. Embedded at compile time so the About data can
/// never drift from the binary's own dependency list or package metadata.
const MANIFEST: &str = include_str!("../Cargo.toml");

static INFO: LazyLock<AboutInfo> = LazyLock::new(|| {
    let packages = parse_dependencies(MANIFEST);
    let gpui_version = packages
        .iter()
        .find(|package| package.name == "gpui")
        .map(|package| package.version.clone());
    AboutInfo {
        product_name: PRODUCT_NAME,
        version: VERSION,
        description: DESCRIPTION,
        license: LICENSE,
        rust_version: RUST_VERSION,
        target: BUILD_TARGET,
        profile: BUILD_PROFILE,
        gpui_version,
        packages,
    }
});

/// The aggregated About data, built once on first access.
pub fn info() -> &'static AboutInfo {
    &INFO
}

/// Parse only the `[dependencies]` table of a Cargo manifest.
///
/// Runtime dependencies are what the About window lists; `[build-dependencies]`,
/// `[dev-dependencies]` and every other table are deliberately ignored (they are not shipped in
/// the binary). Supports the two shapes a dependency can take: a version string
/// (`gpui = "0.2.2"`) and an inline table (`rbook = { version = "0.7.12", ... }`), including
/// inline tables that Cargo allows to span several lines. Path/git dependencies without a
/// `version` key are kept as `"unknown"` rather than silently dropped.
///
/// The `[dependencies.name]` sub-table form is not handled: this manifest does not use it, and a
/// static About list has no need for the extra parsing state.
pub fn parse_dependencies(manifest: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut in_dependencies = false;
    // Name plus the accumulated spec of an inline table spanning several lines.
    let mut pending: Option<(String, String)> = None;

    for raw_line in manifest.lines() {
        let line = strip_comment(raw_line).trim();

        if let Some((name, mut spec)) = pending.take() {
            if !line.is_empty() {
                spec.push(' ');
                spec.push_str(line);
            }
            if inline_table_depth(&spec) > 0 {
                pending = Some((name, spec));
            } else {
                push_package(&mut packages, &name, &spec);
            }
            continue;
        }

        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') {
            let section = line.trim_start_matches('[').trim_end_matches(']').trim();
            in_dependencies = section == "dependencies";
            continue;
        }

        if !in_dependencies {
            continue;
        }

        let Some((name, spec)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        let spec = spec.trim();
        if name.is_empty() || spec.is_empty() {
            continue;
        }
        if inline_table_depth(spec) > 0 {
            pending = Some((name.to_string(), spec.to_string()));
        } else {
            push_package(&mut packages, name, spec);
        }
    }

    // A truncated manifest can end mid-table; keep the partial row instead of panicking or
    // dropping it, matching the module's forgiving-by-design stance.
    if let Some((name, spec)) = pending {
        push_package(&mut packages, &name, &spec);
    }

    packages
}

fn push_package(packages: &mut Vec<Package>, name: &str, spec: &str) {
    packages.push(Package {
        name: name.to_string(),
        version: dependency_version(spec).unwrap_or("unknown").to_string(),
    });
}

/// Extract the version requirement from a dependency spec, either a bare string or an inline
/// table's `version` field (which is not necessarily the first field).
fn dependency_version(spec: &str) -> Option<&str> {
    let spec = spec.trim();
    if !spec.starts_with('{') {
        return unquote(spec);
    }
    let body = spec.trim_start_matches('{').trim_end_matches('}');
    for field in split_top_level(body) {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        if key.trim() == "version" {
            return unquote(value);
        }
    }
    None
}

/// Strip a trailing `# comment`, ignoring `#` inside quoted strings.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == '#' {
            return &line[..index];
        }
    }
    line
}

/// Brace depth of `text`, ignoring braces inside quoted strings. Used to keep reading an inline
/// table that spans several lines.
fn inline_table_depth(text: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Split on commas that are not inside a string or a nested `[...]`/`{...}`. Version requirements
/// may contain commas (`">=1.0, <2"`) and feature lists are arrays, so a plain `split(',')` would
/// corrupt real entries.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// The content of a double-quoted TOML basic string.
fn unquote(value: &str) -> Option<&str> {
    let value = value.trim();
    let rest = value.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}
