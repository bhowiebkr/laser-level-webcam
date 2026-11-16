//! The licences of the packages that a build puts into the programs.
//!
//! `cargo metadata` gives the packages for the targets. This follows the normal dependencies of
//! the window app and the `laser` tool (only the window app for the browser), and writes one text
//! with each package's name, version and licence, then the text of every licence file in the
//! packages. Identical texts show once.
//! A package with no licence file gets the standard text of a licence that it allows. A package
//! with no file and no such licence stops the build, so a new dependency gets a look.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The programs whose packages count. The browser page holds only the window app.
fn roots(targets: &[String]) -> &'static [&'static str] {
    if !targets.is_empty() && targets.iter().all(|t| t.starts_with("wasm32")) {
        &["laser-level-webcam"]
    } else {
        &["laser-level-webcam", "laser"]
    }
}

/// The standard text for a package that has no licence file, by SPDX name, in the order of
/// choice. Apache-2.0 comes first, as its text names no copyright holder.
const STANDARD: [(&str, &str); 3] = [
    ("Apache-2.0", include_str!("../licenses/Apache-2.0.txt")),
    ("BSL-1.0", include_str!("../licenses/BSL-1.0.txt")),
    ("MIT", include_str!("../licenses/MIT.txt")),
];

/// Folders that hold code or test data, not licences.
const SKIP_DIRS: [&str; 5] = ["src", "tests", "benches", "examples", "target"];

struct Package {
    name: String,
    version: String,
    license: String,
    /// (where the text came from, the text)
    texts: Vec<(String, String)>,
}

/// The target that rustc builds for by default.
pub fn host() -> Result<String, String> {
    let out = Command::new("rustc").arg("-vV").output().map_err(|e| format!("running rustc: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc -vV gave no host".into())
}

/// The whole text for these targets.
pub fn text(root: &Path, targets: &[String]) -> Result<String, String> {
    let packages = packages(root, targets)?;
    let mut out = format!(
        "Third-party licences: Laser Level Webcam {} for {}\n\n\
         Laser Level Webcam is under the MIT licence (the LICENSE file). Its programs contain\n\
         these {} packages. Each package keeps its own licence. The licence texts follow the list.\n\n",
        env!("CARGO_PKG_VERSION"),
        targets.join(", "),
        packages.len()
    );
    let width = packages.iter().map(|p| p.name.len() + p.version.len() + 1).max().unwrap_or(0);
    for p in &packages {
        out += &format!("{:<width$}  {}\n", format!("{} {}", p.name, p.version), p.license);
    }

    // Each text once, with every package that has it, in the order of the list.
    let mut order: Vec<&str> = Vec::new();
    let mut users: HashMap<&str, Vec<String>> = HashMap::new();
    for p in &packages {
        for (from, text) in &p.texts {
            let list = users.entry(text.as_str()).or_insert_with(|| {
                order.push(text.as_str());
                Vec::new()
            });
            list.push(format!("{} {} ({from})", p.name, p.version));
        }
    }
    let rule = "=".repeat(80);
    for text in order {
        out += &format!("\n{rule}\n{}\n{}\n\n{text}\n", users[text].join(",\n"), "-".repeat(80));
    }
    Ok(out)
}

fn packages(root: &Path, targets: &[String]) -> Result<Vec<Package>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["metadata", "--format-version", "1", "--locked"]).current_dir(root);
    for t in targets {
        cmd.args(["--filter-platform", t]);
    }
    let out = cmd.output().map_err(|e| format!("running cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))?;
    let all = meta["packages"].as_array().ok_or("cargo metadata gave no packages")?;
    let by_id: HashMap<&str, &Value> = all.iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .ok_or("cargo metadata gave no dependency graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();

    // The normal dependencies (not build or dev ones) of the programs, at any depth.
    let roots = roots(targets);
    let mut stack: Vec<&str> = all
        .iter()
        .filter(|p| roots.contains(&p["name"].as_str().unwrap_or("")))
        .filter_map(|p| p["id"].as_str())
        .collect();
    if stack.len() != roots.len() {
        return Err(format!("cargo metadata did not list each of {roots:?}"));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let deps = nodes.get(id).and_then(|n| n["deps"].as_array()).map(Vec::as_slice).unwrap_or_default();
        for d in deps {
            let normal = d["dep_kinds"].as_array().is_some_and(|k| k.iter().any(|k| k["kind"].is_null()));
            if let (true, Some(pkg)) = (normal, d["pkg"].as_str()) {
                stack.push(pkg);
            }
        }
    }

    let mut list: BTreeMap<(String, String), Package> = BTreeMap::new();
    let mut missing = Vec::new();
    for id in seen {
        let p = by_id[id];
        // The workspace's own packages have no source.
        if p["source"].is_null() {
            continue;
        }
        let s = |k: &str| p[k].as_str().unwrap_or_default().to_owned();
        let (name, version, license) = (s("name"), s("version"), s("license"));
        let dir =
            Path::new(p["manifest_path"].as_str().unwrap_or_default()).parent().unwrap_or(Path::new("")).to_owned();
        let mut files = licence_files(&dir);
        if let Some(f) = p["license_file"].as_str().map(|f| dir.join(f))
            && !files.contains(&f)
        {
            files.push(f);
        }
        let mut texts = Vec::new();
        for f in files {
            let raw = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            let label = f.strip_prefix(&dir).unwrap_or(&f).to_string_lossy().replace('\\', "/");
            texts.push((label, clean(&String::from_utf8_lossy(&raw))));
        }
        if texts.is_empty() {
            match standard(&license) {
                Some(found) => {
                    let authors: Vec<&str> = p["authors"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    let by =
                        if authors.is_empty() { String::new() } else { format!("; authors: {}", authors.join(", ")) };
                    for (id, text) in found {
                        texts
                            .push((format!("no licence file in the package; the standard {id} text{by}"), clean(text)));
                    }
                }
                None => missing.push(format!("{name} {version} ({license})")),
            }
        }
        list.insert((name.clone(), version.clone()), Package { name, version, license, texts });
    }
    if !missing.is_empty() {
        return Err(format!(
            "these packages have no licence file, and pack has no standard text for their licence: {}. \
             Add the text to tools/pack/licenses and to STANDARD in licenses.rs",
            missing.join(", ")
        ));
    }
    Ok(list.into_values().collect())
}

/// The licence files in a package folder: at the top, and in folders up to two down (such as
/// the fonts of a font package, whose notes sit next to them).
fn licence_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        // A LICENSES folder, or the notes next to the fonts of a font package.
        let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
        let notes_dir = depth > 0
            && (dir_name.contains("licen") || entries.iter().any(|p| matches!(ext(p).as_str(), "ttf" | "otf")));
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
            if p.is_dir() {
                if depth < 2 && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    walk(&p, depth + 1, out);
                }
                continue;
            }
            // Names such as LICENSE.APACHE or COPYING.LESSER: anything that is not code or data.
            if matches!(ext(&p).as_str(), "rs" | "toml" | "lock" | "json" | "ttf" | "otf" | "png" | "html" | "sh") {
                continue;
            }
            let named = ["license", "licence", "copying", "notice", "unlicense", "copyright"]
                .iter()
                .any(|k| name.starts_with(k))
                || (depth > 0
                    && (name.contains("license")
                        || name.contains("licence")
                        || name == "ofl.txt"
                        || name == "ufl.txt"));
            if named || (notes_dir && matches!(ext(&p).as_str(), "" | "txt" | "md")) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// LF line ends, no BOM, no blank lines at the ends.
fn clean(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_matches('\n').to_owned()
}

/// The standard texts that cover an SPDX expression: one licence of a choice (`A OR B`, `A/B`),
/// or every licence when the expression has `AND`.
fn standard(expr: &str) -> Option<Vec<(&'static str, &'static str)>> {
    let ids: Vec<&str> = expr
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
        .filter(|t| !t.is_empty() && !matches!(*t, "OR" | "AND" | "WITH"))
        .collect();
    let find = |id: &str| STANDARD.iter().find(|(s, _)| *s == id).copied();
    if expr.contains(" AND ") {
        ids.iter().map(|id| find(id)).collect()
    } else {
        STANDARD.iter().find(|(s, _)| ids.contains(s)).map(|&one| vec![one])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_takes_one_known_licence() {
        assert_eq!(standard("MIT OR Apache-2.0").map(|v| v[0].0), Some("Apache-2.0"));
        assert_eq!(standard("MIT/Apache-2.0").map(|v| v[0].0), Some("Apache-2.0"));
        assert_eq!(standard("MIT OR BSL-1.0").map(|v| v[0].0), Some("BSL-1.0"));
        assert_eq!(standard("BSL-1.0").map(|v| v[0].0), Some("BSL-1.0"));
        assert_eq!(standard("MIT").map(|v| v[0].0), Some("MIT"));
        assert!(standard("ISC").is_none());
        assert!(standard("Apache-2.0 AND ISC").is_none());
        assert_eq!(standard("Apache-2.0 AND MIT").map(|v| v.len()), Some(2));
    }

    #[test]
    fn the_browser_page_counts_only_the_window_app() {
        let t = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(roots(&t(&["wasm32-unknown-unknown"])), ["laser-level-webcam"]);
        assert_eq!(roots(&t(&["x86_64-pc-windows-msvc"])), ["laser-level-webcam", "laser"]);
        assert_eq!(roots(&t(&["aarch64-apple-darwin", "x86_64-apple-darwin"])), ["laser-level-webcam", "laser"]);
        assert_eq!(roots(&[]), ["laser-level-webcam", "laser"]);
    }

    #[test]
    fn clean_ends_and_line_breaks() {
        assert_eq!(clean("\u{feff}\n\nA  \r\nB\r\n\n"), "A\nB");
    }
}
