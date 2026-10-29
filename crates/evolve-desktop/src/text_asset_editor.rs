//! In-viewer UTF-8 text editor for extracted XML, Lua source, and TXT.
//! Only authenticated editable Modding projects can save; imported Asset
//! Library files and any installed game PAKs are never written here.
use crate::modding_projects::{self, ProjectRow};
use iced::widget::text_editor;
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;

pub fn compatible(member: &str) -> bool {
    let name = member.to_ascii_lowercase();
    [".xml", ".lua", ".txt"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

pub struct Document {
    pub member: String,
    pub content: text_editor::Content,
    pub editable: bool,
    pub project: Option<ProjectRow>,
    pub projects_root: PathBuf,
    original: Vec<u8>,
    saved: String,
}

impl Document {
    pub fn dirty(&self) -> bool {
        self.content.text() != self.saved
    }

    pub fn open(
        inventory_root: &Path,
        member: &str,
        project: Option<ProjectRow>,
        projects_root: PathBuf,
    ) -> Result<Self, String> {
        if !compatible(member) || !modding_projects::valid_text_member(member) {
            return Err("Only safe extracted .xml, .lua and .txt members can be previewed.".into());
        }
        let root = fs::canonicalize(inventory_root).map_err(|e| e.to_string())?;
        let source = inventory_root.join(member);
        let meta = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        if !meta.file_type().is_file() || meta.len() > MAX_TEXT_BYTES {
            return Err("Text editor supports regular files up to 2 MiB; larger files require an external editor.".into());
        }
        let resolved = fs::canonicalize(&source).map_err(|e| e.to_string())?;
        if !resolved.starts_with(&root) {
            return Err("Selected asset resolves outside its indexed extraction root.".into());
        }
        let raw = fs::read(&source).map_err(|e| e.to_string())?;
        if raw.len() as u64 > MAX_TEXT_BYTES || raw.contains(&0) {
            return Err("This asset is binary or too large for the text editor. Compiled Lua bytecode cannot be edited as Lua source.".into());
        }
        let data = std::str::from_utf8(&raw).map_err(|_| {
            "This file is not UTF-8 text. Compiled Lua and binary CryXML must be decoded before editing.".to_owned()
        })?;
        let saved = data.to_owned();
        let editable = project.as_ref().is_some_and(|project| {
            fs::canonicalize(project.workspace.join("files"))
                .is_ok_and(|project_root| project_root == root)
        });
        Ok(Self {
            member: member.to_owned(),
            content: text_editor::Content::with_text(data),
            editable,
            project: if editable { project } else { None },
            projects_root,
            original: raw,
            saved,
        })
    }

    pub fn save(&mut self) -> Result<bool, String> {
        if !self.editable {
            return Err(
                "Asset Library originals are read-only. Create an editable Modding project first."
                    .into(),
            );
        }
        let project = self
            .project
            .as_ref()
            .ok_or("No editable Modding project is attached")?;
        let edited = self.content.text();
        if edited.len() as u64 > MAX_TEXT_BYTES {
            return Err("Edited document exceeds 2 MiB text editor limit.".into());
        }
        if !self.dirty() {
            return Ok(false);
        }
        let changed = modding_projects::save_text_member(
            project,
            &self.projects_root,
            &self.member,
            &self.original,
            &edited,
        )?;
        if changed {
            self.original = edited.as_bytes().to_vec();
            self.saved = edited;
        }
        Ok(changed)
    }

    pub fn discard(&mut self) {
        self.content = text_editor::Content::with_text(&self.saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn recognizes_only_supported_text_extensions() {
        for name in ["libs/A.xml", "scripts/ai.LUA", "notes/readme.Txt"] {
            assert!(compatible(name));
        }
        for name in [
            "scripts/ai.luac",
            "libs/A.xmlb",
            "material.dds",
            "name.xml.exe",
        ] {
            assert!(!compatible(name));
        }
    }

    #[test]
    fn imported_asset_is_read_only_and_compiled_lua_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("hello.txt"), "Hello, world").unwrap();
        let mut doc = Document::open(&root, "hello.txt", None, temp.path().to_path_buf()).unwrap();
        assert!(!doc.editable);
        assert!(!doc.dirty());
        assert!(doc.save().is_err());
        fs::write(root.join("compiled.lua"), [0x1b, b'L', b'J', 0, 0xff]).unwrap();
        assert!(Document::open(&root, "compiled.lua", None, temp.path().to_path_buf()).is_err());
        assert_eq!(fs::read(root.join("hello.txt")).unwrap(), b"Hello, world");
        assert!(Document::open(&root, "../outside.txt", None, temp.path().to_path_buf()).is_err());
    }

    #[test]
    fn edits_are_backed_up_and_guard_against_external_changes() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let workspace = projects.join("MyMod");
        let root = workspace.join("files");
        let member = "Libs/GameRules/ArenaMode.xml";
        let file = root.join(member);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = b"<Arena rounds=\"2\"/>".to_vec();
        fs::write(&file, &original).unwrap();
        fs::write(workspace.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "entries": [{"path": member, "format": "raw", "sha256":format!("{:x}",Sha256::digest(&original))}]
        })).unwrap()).unwrap();
        let project = ProjectRow {
            label: "My Mod".into(),
            target: "libs.pak".into(),
            source_build: "Stage 2".into(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        let mut doc =
            Document::open(&root, member, Some(project.clone()), projects.clone()).unwrap();
        assert!(doc.editable);
        // Simulate a user editing the text-editor buffer.
        doc.content = text_editor::Content::with_text("<Arena rounds=\"1\"/>");
        assert!(doc.dirty());
        assert!(doc.save().unwrap());
        assert!(!doc.dirty());
        assert_eq!(fs::read(&file).unwrap(), b"<Arena rounds=\"1\"/>");
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        // Stale edits may never silently overwrite external workspace changes.
        doc.content = text_editor::Content::with_text("<Arena rounds=\"3\"/>");
        fs::write(&file, b"external change").unwrap();
        assert!(doc.save().is_err());
        assert_eq!(fs::read(&file).unwrap(), b"external change");
    }
}
