use epub_reader::about::{
    BUILD_PROFILE, BUILD_TARGET, DESCRIPTION, LICENSE, PRODUCT_NAME, Package, RUST_VERSION, VERSION,
    info, parse_dependencies,
};

fn version_of<'a>(packages: &'a [Package], name: &str) -> Option<&'a str> {
    packages
        .iter()
        .find(|package| package.name == name)
        .map(|package| package.version.as_str())
}

#[test]
fn parses_manifest_runtime_dependencies() {
    let packages = parse_dependencies(include_str!("../Cargo.toml"));

    assert_eq!(version_of(&packages, "gpui"), Some("0.2.2"));
    assert_eq!(version_of(&packages, "rbook"), Some("0.7.12"));
    assert_eq!(version_of(&packages, "serde"), Some("1.0.229"));
    // `scraper` is also a dev-dependency, but it is a runtime dependency first.
    assert_eq!(version_of(&packages, "scraper"), Some("0.29.0"));
    // `embed-resource` exists only in [build-dependencies]: parsing must stop at that section.
    assert_eq!(version_of(&packages, "embed-resource"), None);
    assert!(packages.iter().all(|package| !package.name.is_empty()));
    assert!(packages.iter().all(|package| !package.version.is_empty()));

    // Manifest order is preserved so the About list is not reshuffled.
    let position = |name: &str| {
        packages
            .iter()
            .position(|package| package.name == name)
            .unwrap()
    };
    assert!(position("gpui") < position("rbook"));
    assert!(position("rbook") < position("smol"));
}

const SAMPLE_MANIFEST: &str = r#"
# [package] comes before [dependencies] and must not be scanned.
[package]
name = "sample"
version = "3.2.1"

[dependencies]
# A full-line comment is skipped.
foo = "1.2.3"
bar = { version = "4.5.6", default-features = false, features = ["x", "y"] } # trailing comment
baz = { features = ["z"], version = "7.8.9" }
ranged = { version = ">=1.0, <2", default-features = false }
local = { path = "../local" }
multi = {
    version = "9.9.9",
    default-features = false,
}

[build-dependencies]
embed-resource = "3.0.12"

[dev-dependencies]
insta = "1.42.0"

[profile.release]
opt-level = 3
"#;

#[test]
fn parser_handles_inline_tables_comments_and_sections() {
    let packages = parse_dependencies(SAMPLE_MANIFEST);
    assert_eq!(
        packages,
        vec![
            Package {
                name: "foo".into(),
                version: "1.2.3".into(),
            },
            Package {
                name: "bar".into(),
                version: "4.5.6".into(),
            },
            Package {
                name: "baz".into(),
                version: "7.8.9".into(),
            },
            // The comma inside the requirement string must not split the inline table.
            Package {
                name: "ranged".into(),
                version: ">=1.0, <2".into(),
            },
            // A path dependency has no declared version; it stays listed rather than vanishing.
            Package {
                name: "local".into(),
                version: "unknown".into(),
            },
            // Inline tables are allowed to span several lines.
            Package {
                name: "multi".into(),
                version: "9.9.9".into(),
            },
        ]
    );
}

#[test]
fn info_matches_compiled_package_metadata() {
    let about = info();

    assert_eq!(about.product_name, PRODUCT_NAME);
    assert_eq!(about.product_name, "Another Epub Reader");
    assert_eq!(about.version, VERSION);
    assert_eq!(about.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(about.version, "0.1.0");
    assert_eq!(about.description, DESCRIPTION);
    assert!(!about.description.is_empty());
    assert_eq!(about.license, LICENSE);
    assert_eq!(about.license, "MIT");

    assert_eq!(about.rust_version, RUST_VERSION);
    assert_ne!(about.rust_version, "unknown");
    assert!(
        about
            .rust_version
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit()),
        "rust_version should start with a digit, got {:?}",
        about.rust_version
    );

    assert_eq!(about.target, BUILD_TARGET);
    assert_ne!(about.target, "unknown");
    if cfg!(windows) {
        assert_eq!(about.target, "x86_64-pc-windows-msvc");
    }

    let expected_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    assert_eq!(about.profile, BUILD_PROFILE);
    assert_eq!(about.profile, expected_profile);

    assert_eq!(about.gpui_version.as_deref(), Some("0.2.2"));
    assert_eq!(version_of(&about.packages, "gpui"), Some("0.2.2"));
    assert!(!about.packages.is_empty());

    // `LazyLock` keeps one instance for the whole process.
    assert!(std::ptr::eq(info(), info()));
}
