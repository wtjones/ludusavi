use crate::{path::StrictPath, prelude::ScannedFile};

const SAFE: &str = "_";

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IndividualMapping {
    pub name: String,
    #[serde(serialize_with = "crate::serialization::ordered_map")]
    pub drives: std::collections::HashMap<String, String>,
}

impl IndividualMapping {
    pub fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    fn reversed_drives(&self) -> std::collections::HashMap<String, String> {
        self.drives.iter().map(|(k, v)| (v.to_owned(), k.to_owned())).collect()
    }

    pub fn drive_folder_name(&mut self, drive: &str) -> String {
        let reversed = self.reversed_drives();
        match reversed.get::<str>(&drive) {
            Some(mapped) => mapped.to_string(),
            None => {
                let mut key = "drive-1".to_string();
                for n in 2..1000 {
                    if !self.drives.contains_key(&key) {
                        self.drives.insert(key.to_string(), drive.to_string());
                        break;
                    }
                    key = format!("drive-{}", n);
                }
                key
            }
        }
    }

    pub fn save(&self, file: &StrictPath) {
        std::fs::write(file.interpret(), self.serialize().as_bytes()).unwrap();
    }

    pub fn serialize(&self) -> String {
        serde_yaml::to_string(&self).unwrap()
    }

    pub fn load(file: &StrictPath) -> Result<Self, ()> {
        if !file.is_file() {
            return Err(());
        }
        let content = std::fs::read_to_string(&file.interpret()).unwrap();
        Self::load_from_string(&content)
    }

    pub fn load_from_string(content: &str) -> Result<Self, ()> {
        match serde_yaml::from_str(&content) {
            Ok(x) => Ok(x),
            Err(_) => Err(()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OverallMapping {
    pub games: std::collections::HashMap<String, OverallMappingGame>,
}

#[derive(Clone, Debug, Default)]
pub struct OverallMappingGame {
    pub drives: std::collections::HashMap<String, String>,
    pub base: StrictPath,
}

impl OverallMapping {
    pub fn load(base: &StrictPath) -> Self {
        let mut overall = Self::default();

        for game_dir in walkdir::WalkDir::new(base.interpret())
            .max_depth(1)
            .follow_links(false)
            .into_iter()
            .skip(1) // the base path itself
            .filter_map(|e| e.ok())
            .filter(|x| x.file_type().is_dir())
        {
            let individual_file = &mut game_dir.path().to_path_buf();
            individual_file.push("mapping.yaml");
            if individual_file.is_file() {
                let game = match IndividualMapping::load(&StrictPath::from_std_path_buf(&individual_file)) {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                overall.games.insert(
                    game.name,
                    OverallMappingGame {
                        base: StrictPath::from_std_path_buf(&game_dir.path().to_path_buf()),
                        drives: game.drives,
                    },
                );
            }
        }

        overall
    }
}

#[derive(Clone, Debug, Default)]
pub struct BackupLayout {
    pub base: StrictPath,
    pub mapping: OverallMapping,
}

impl BackupLayout {
    pub fn new(base: StrictPath) -> Self {
        let mapping = OverallMapping::load(&base);
        Self { base, mapping }
    }

    fn escape_folder_name(name: &str) -> String {
        if name == "." || name == ".." {
            name.replace(".", SAFE)
        } else {
            name.replace("\\", SAFE)
                .replace("/", SAFE)
                .replace(":", SAFE)
                .replace("*", SAFE)
                .replace("?", SAFE)
                .replace("\"", SAFE)
                .replace("<", SAFE)
                .replace(">", SAFE)
                .replace("|", SAFE)
                .replace("\0", SAFE)
        }
    }

    fn generate_total_rename() -> String {
        format!("ludusavi-renamed-{}", rand::random::<u16>())
    }

    pub fn game_folder(&self, game_name: &str) -> StrictPath {
        match self.mapping.games.get::<str>(&game_name) {
            Some(game) => game.base.clone(),
            None => {
                let mut safe_name = Self::escape_folder_name(game_name);

                if safe_name.matches(SAFE).count() == safe_name.len() {
                    // It's unreadable now, so do a total rename.
                    safe_name = Self::generate_total_rename();

                    let mut attempts = 0;
                    while self.base.joined(&safe_name).exists() {
                        // Regenerate to avoid conflicts, although conflicts are
                        // extremely unlikely because total renames are rare.
                        safe_name = Self::generate_total_rename();
                        attempts += 1;
                        if attempts > 1000 {
                            // Realistically, this will never happen.
                            safe_name = "ludusavi-renamed-collision".to_owned();
                            break;
                        }
                    }
                }

                self.base.joined(&safe_name)
            }
        }
    }

    pub fn game_file(
        &self,
        game_folder: &StrictPath,
        original_file: &StrictPath,
        mapping: &mut IndividualMapping,
    ) -> StrictPath {
        let (drive, plain_path) = original_file.split_drive();
        let drive_folder = mapping.drive_folder_name(&drive);
        StrictPath::relative(
            format!("{}/{}", drive_folder, plain_path),
            Some(game_folder.interpret()),
        )
    }

    pub fn game_mapping_file(&self, game_folder: &StrictPath) -> StrictPath {
        game_folder.joined("mapping.yaml")
    }

    #[allow(dead_code)]
    pub fn game_registry_file(&self, game_folder: &StrictPath) -> StrictPath {
        game_folder.joined("registry.yaml")
    }

    pub fn restorable_files(
        &self,
        game_name: &str,
        game_folder: &StrictPath,
    ) -> std::collections::HashSet<ScannedFile> {
        let mut files = std::collections::HashSet::new();
        for drive_dir in walkdir::WalkDir::new(game_folder.interpret())
            .max_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let raw_drive_dir = drive_dir.path().display().to_string();
            let drive_mapping = match self.mapping.games.get::<str>(&game_name) {
                Some(x) => match x.drives.get::<str>(&drive_dir.file_name().to_string_lossy()) {
                    Some(y) => y,
                    None => continue,
                },
                None => continue,
            };

            for file in walkdir::WalkDir::new(drive_dir.path())
                .max_depth(100)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|x| x.file_type().is_file())
            {
                let raw_file = file.path().display().to_string();
                let original_path = Some(StrictPath::new(raw_file.replace(&raw_drive_dir, drive_mapping)));
                files.insert(ScannedFile {
                    path: StrictPath::new(raw_file),
                    size: match file.metadata() {
                        Ok(m) => m.len(),
                        _ => 0,
                    },
                    original_path,
                });
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn repo() -> String {
        env!("CARGO_MANIFEST_DIR").to_string()
    }

    fn layout() -> BackupLayout {
        BackupLayout::new(StrictPath::new(repo()))
    }

    #[test]
    fn can_find_game_folder_with_matching_name() {
        assert_eq!(
            StrictPath::new(if cfg!(target_os = "windows") {
                format!("\\\\?\\{}/game1", repo())
            } else {
                format!("{}/game1", repo())
            }),
            layout().game_folder("game1")
        );
    }

    #[test]
    fn can_find_game_folder_with_rename() {
        assert_eq!(
            StrictPath::new(format!("{}/game3-renamed", repo())),
            layout().game_folder("game3")
        );
    }

    #[test]
    fn can_find_game_folder_that_does_not_exist() {
        assert_eq!(
            StrictPath::new(format!("{}/nonexistent", repo())),
            layout().game_folder("nonexistent")
        );
    }
}
