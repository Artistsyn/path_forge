//! The agent skill PathForge ships: one SKILL.md (the open Agent Skills format), compiled into the
//! binaries so every install carries the guide that matches its tools. `pf skill install` writes it
//! where each coding agent looks; `pf_mcp` also serves it as an MCP prompt and resource, so an agent
//! connected over MCP can read it without installing anything.

use std::path::{Path, PathBuf};

/// The skill, exactly as in `skills/pathforge/SKILL.md`.
pub const SKILL_MD: &str = include_str!("../skills/pathforge/SKILL.md");
pub const NAME: &str = "pathforge";
/// Where MCP clients read it as a resource.
pub const RESOURCE_URI: &str = "pathforge://skill/SKILL.md";

/// The one-line description from the skill's front matter.
pub fn description() -> &'static str {
    SKILL_MD.lines().find_map(|l| l.strip_prefix("description: ")).unwrap_or("")
}

/// Where one agent family loads skills from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code: `.claude/skills/`.
    Claude,
    /// The shared Agent Skills folder: `.agents/skills/` (Codex; Copilot, Cursor and others read it too).
    Agents,
    /// GitHub Copilot: `.github/skills/` (projects only).
    Copilot,
    /// Cursor: `.cursor/skills/`.
    Cursor,
}

impl Agent {
    pub const ALL: [Agent; 4] = [Agent::Claude, Agent::Agents, Agent::Copilot, Agent::Cursor];
    /// Claude Code reads only its own folder; nearly everything else reads `.agents`.
    pub const DEFAULT: [Agent; 2] = [Agent::Claude, Agent::Agents];

    pub fn name(self) -> &'static str {
        match self { Agent::Claude => "claude", Agent::Agents => "agents", Agent::Copilot => "copilot", Agent::Cursor => "cursor" }
    }
    pub fn parse(s: &str) -> Result<Vec<Agent>, String> {
        let mut out = Vec::new();
        for part in s.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
            match part.as_str() {
                "all" => out.extend(Agent::ALL),
                "claude" => out.push(Agent::Claude),
                "agents" | "codex" | "gemini" | "generic" => out.push(Agent::Agents),
                "copilot" | "github" => out.push(Agent::Copilot),
                "cursor" => out.push(Agent::Cursor),
                other => return Err(format!("unknown agent '{other}' (claude, agents/codex, copilot, cursor, all)")),
            }
        }
        out.dedup();
        Ok(out)
    }
    /// The skill file under a project root or a home folder.
    pub fn path(self, root: &Path, user: bool) -> Result<PathBuf, String> {
        let dir = match (self, user) {
            (Agent::Claude, _) => ".claude",
            (Agent::Agents, _) => ".agents",
            (Agent::Cursor, _) => ".cursor",
            (Agent::Copilot, false) => ".github",
            (Agent::Copilot, true) => return Err("Copilot has no per-user skill folder; install into a project, or use 'agents' (Copilot reads .agents/skills)".into()),
        };
        Ok(root.join(dir).join("skills").join(NAME).join("SKILL.md"))
    }
}

/// Beside each installed copy: the revision of what was written, so an untouched copy can be upgraded
/// and an edited one recognised.
const STAMP: &str = ".installed-revision";

/// What happened to one target.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed { Written(PathBuf), Unchanged(PathBuf), Kept(PathBuf) }

/// Write the skill for each agent under `root`. A file someone has edited is kept unless `force`,
/// so local changes are never lost silently.
pub fn install(agents: &[Agent], root: &Path, user: bool, force: bool) -> Result<Vec<Installed>, String> {
    let mut out = Vec::new();
    for &a in agents {
        let path = a.path(root, user)?;
        let dir = path.parent().unwrap().to_path_buf();
        let stamp = dir.join(STAMP);
        match std::fs::read_to_string(&path) {
            Ok(old) if old == SKILL_MD => { out.push(Installed::Unchanged(path)); continue; }
            Ok(old) => {
                let untouched = std::fs::read_to_string(&stamp).map(|r| r.trim() == crate::scene::revision(&old)).unwrap_or(false);
                if !untouched && !force { out.push(Installed::Kept(path)); continue; }
            }
            Err(_) => {}
        }
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        std::fs::create_dir_all(&dir).map_err(err)?;
        let part = path.with_extension("md.part");
        std::fs::write(&part, SKILL_MD).and_then(|_| std::fs::rename(&part, &path)).map_err(err)?;
        std::fs::write(&stamp, crate::scene::revision(SKILL_MD)).map_err(err)?;
        out.push(Installed::Written(path));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_follows_the_agent_skills_format() {
        // Front matter: name matches its folder (lowercase, hyphens), description within 1024 chars.
        assert!(SKILL_MD.starts_with("---\nname: pathforge\n"));
        let d = description();
        assert!(!d.is_empty() && d.len() <= 1024, "description is {} chars", d.len());
        assert!(NAME.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
        assert!(SKILL_MD[4..].contains("\n---\n"), "front matter is not closed");
    }

    #[test]
    fn every_tool_the_skill_names_exists() {
        let tools: Vec<String> = crate::mcp::tool_names();
        let mut named: Vec<&str> = SKILL_MD.match_indices("pf_").map(|(i, _)| {
            let rest = &SKILL_MD[i..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
            &rest[..end]
        // pf_runtime_* are the runtime's C functions, not tools.
        }).filter(|n| *n != "pf_mcp" && *n != "pf_" && !n.starts_with("pf_runtime_")).collect();
        named.sort();
        named.dedup();
        for n in &named { assert!(tools.iter().any(|t| t == n), "SKILL.md names {n}, which pf_mcp does not have"); }
        // And the guide covers the tools an agent needs most.
        for t in ["pf_new_scene", "pf_edit_scene", "pf_contact_sheet", "pf_check_loop", "pf_export", "pf_camera"] {
            assert!(named.contains(&t), "SKILL.md never mentions {t}");
        }
    }

    #[test]
    fn install_writes_each_agent_folder_and_keeps_edited_copies() {
        let root = std::env::temp_dir().join(format!("pf_skill_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = install(&Agent::DEFAULT, &root, false, false).unwrap();
        assert_eq!(r.len(), 2);
        assert!(root.join(".claude/skills/pathforge/SKILL.md").exists());
        assert!(root.join(".agents/skills/pathforge/SKILL.md").exists());
        // Again: nothing to do.
        assert!(install(&Agent::DEFAULT, &root, false, false).unwrap().iter().all(|i| matches!(i, Installed::Unchanged(_))));
        // An older copy we wrote and nobody touched is upgraded in place.
        let p = root.join(".claude/skills/pathforge/SKILL.md");
        let older = "---\nname: pathforge\n---\nan older version";
        std::fs::write(&p, older).unwrap();
        std::fs::write(p.with_file_name(STAMP), crate::scene::revision(older)).unwrap();
        assert!(matches!(install(&[Agent::Claude], &root, false, false).unwrap()[0], Installed::Written(_)));
        // A user's own edit survives a reinstall unless forced.
        std::fs::write(&p, format!("{SKILL_MD}\nmy notes")).unwrap();
        assert!(matches!(install(&[Agent::Claude], &root, false, false).unwrap()[0], Installed::Kept(_)));
        assert!(matches!(install(&[Agent::Claude], &root, false, true).unwrap()[0], Installed::Written(_)));
        assert!(Agent::Copilot.path(&root, true).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
