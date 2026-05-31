use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamGameIdentity {
    pub app_id: u32,
    pub name: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Clone)]
pub struct SteamGameTitleResolver {
    library_roots: Vec<PathBuf>,
}

impl SteamGameTitleResolver {
    pub fn new(configured_roots: &[PathBuf]) -> Self {
        Self {
            library_roots: discover_library_roots(configured_roots),
        }
    }

    pub fn resolve(&self, app_id: u32) -> SteamGameIdentity {
        SteamGameIdentity {
            app_id,
            name: self.resolve_name(app_id),
            source: "steam_appmanifest",
        }
    }

    pub fn resolve_name(&self, app_id: u32) -> Option<String> {
        for manifest in manifest_paths(&self.library_roots, app_id) {
            let Ok(text) = fs::read_to_string(manifest) else {
                continue;
            };
            if let Some(name) = parse_acf_string_field(&text, "name") {
                if !name.trim().is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }
}

fn discover_library_roots(configured_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    for root in configured_roots {
        roots.insert(root.clone());
    }
    for root in default_steam_roots() {
        roots.insert(root);
    }

    let mut discovered = roots.clone();
    for root in roots {
        let libraryfolders = root.join("steamapps/libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(libraryfolders) {
            for path in parse_library_folder_paths(&text) {
                discovered.insert(path);
            }
        }
    }

    discovered.into_iter().collect()
}

fn default_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".steam/steam"));
    }
    roots.push(PathBuf::from("/home/deck/.local/share/Steam"));
    roots
}

fn manifest_paths(roots: &[PathBuf], app_id: u32) -> Vec<PathBuf> {
    let filename = format!("appmanifest_{app_id}.acf");
    let mut paths = Vec::new();
    for root in roots {
        paths.push(root.join("steamapps").join(&filename));
        paths.push(root.join(&filename));
    }
    paths
}

fn parse_library_folder_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let values = quoted_values(line);
            if values.len() < 2 {
                return None;
            }
            let is_path_field = values[0] == "path";
            let is_legacy_path = values[0]
                .chars()
                .all(|character| character.is_ascii_digit())
                && looks_like_path(&values[1]);
            if is_path_field || is_legacy_path {
                Some(PathBuf::from(&values[1]))
            } else {
                None
            }
        })
        .collect()
}

fn looks_like_path(value: &str) -> bool {
    value.starts_with('/') || value.contains(":\\") || value.contains("/Steam")
}

fn parse_acf_string_field(text: &str, field: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let values = quoted_values(line);
        if values.len() >= 2 && values[0] == field {
            Some(values[1].clone())
        } else {
            None
        }
    })
}

fn quoted_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(inner) = chars.next() {
            match inner {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                '"' => break,
                other => value.push(other),
            }
        }
        values.push(value);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("telemon-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn resolves_game_name_from_appmanifest() {
        let root = temp_dir("steam-appmanifest");
        let steamapps = root.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join("appmanifest_1234.acf"),
            "\"AppState\"\n{\n    \"appid\" \"1234\"\n    \"name\" \"Example Game\"\n}\n",
        )
        .unwrap();

        let resolver = SteamGameTitleResolver::new(&[root.clone()]);
        assert_eq!(
            resolver.resolve_name(1234),
            Some("Example Game".to_string())
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_libraryfolder_paths() {
        let text = "\"libraryfolders\"\n{\n  \"0\"\n  {\n    \"path\" \"/home/deck/.local/share/Steam\"\n  }\n  \"1\" \"/mnt/games/SteamLibrary\"\n}\n";
        let paths = parse_library_folder_paths(text);
        assert!(paths.contains(&PathBuf::from("/home/deck/.local/share/Steam")));
        assert!(paths.contains(&PathBuf::from("/mnt/games/SteamLibrary")));
    }
}
