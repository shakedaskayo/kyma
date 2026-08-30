//! Filesystem skill delivery for the Claude CLI: write skills under a temp
//! workdir's `.claude/skills/` so the spawned `claude` discovers them (it reads
//! `<cwd>/.claude/skills/<name>/SKILL.md` natively). Agent-agnostic entry point;
//! the ClaudeCli engine passes the workdir as `cwd`.

pub struct SkillDoc {
    pub name: String,
    pub body: String,
}

/// Holds the temp workdir; must outlive the spawned CLI (drop cleans it up).
pub struct DeliveredSkills {
    pub workdir: tempfile::TempDir,
}

/// Write `<workdir>/.claude/skills/<name>/SKILL.md` for each skill.
pub async fn deliver_to_workdir(skills: &[SkillDoc]) -> anyhow::Result<DeliveredSkills> {
    let workdir = tempfile::tempdir()?;
    for s in skills {
        let dir = workdir.path().join(".claude/skills").join(&s.name);
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(dir.join("SKILL.md"), &s.body).await?;
    }
    Ok(DeliveredSkills { workdir })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn writes_each_skill_under_claude_skills() {
        let skills = vec![
            SkillDoc { name: "pensieve-dreaming".into(), body: "---\nname: pensieve-dreaming\n---\nbody".into() },
            SkillDoc { name: "pensieve-memory".into(), body: "x".into() },
        ];
        let d = deliver_to_workdir(&skills).await.unwrap();
        let p = d.workdir.path().join(".claude/skills/pensieve-dreaming/SKILL.md");
        assert!(p.exists(), "pensieve-dreaming SKILL.md written");
        assert!(std::fs::read_to_string(&p).unwrap().contains("name: pensieve-dreaming"));
        assert!(d.workdir.path().join(".claude/skills/pensieve-memory/SKILL.md").exists());
    }

    #[tokio::test]
    async fn empty_skill_list_creates_just_the_workdir() {
        let d = deliver_to_workdir(&[]).await.unwrap();
        assert!(d.workdir.path().exists());
    }
}
