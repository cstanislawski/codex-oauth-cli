use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SECTION_SYSTEM: &str = "--- system ---";
const SECTION_USER: &str = "--- user ---";

const DEFAULT_TEMPLATE: &str = "{{prompt}}";
const REVIEW_TEMPLATE: &str = "\
--- system ---
You are a strict code reviewer. Prioritize bugs, regressions, missing tests, and unsafe assumptions.
Keep findings concrete and terse.
--- user ---
Review this change:

{{prompt}}
";
const COMMIT_TEMPLATE: &str = "\
--- system ---
You write compact, high-signal git commit messages.
Return subject line first, blank line, then bullet list when needed.
--- user ---
Write a commit message for:

{{prompt}}
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedTemplate {
    pub system: String,
    pub user: String,
}

pub fn templates_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("templates")
}

pub fn list(config_dir: &Path) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();
    for (name, _) in builtins() {
        names.insert(name.to_string());
    }

    let dir = templates_dir(config_dir);
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                    names.insert(stem.to_string());
                }
            }
        }
    }

    Ok(names.into_iter().collect())
}

pub fn init(config_dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = templates_dir(config_dir);
    fs::create_dir_all(&dir)?;
    let mut created = Vec::new();
    for (name, contents) in builtins() {
        let path = dir.join(format!("{name}.txt"));
        if !path.exists() {
            fs::write(&path, contents)?;
            created.push(path);
        }
    }
    Ok(created)
}

pub fn render(
    config_dir: &Path,
    template_name: Option<&str>,
    prompt: &str,
) -> Result<RenderedTemplate> {
    let template_text = load_template_text(config_dir, template_name)?;
    Ok(parse_and_render(&template_text, prompt))
}

fn load_template_text(config_dir: &Path, template_name: Option<&str>) -> Result<String> {
    let name = template_name.unwrap_or("default");

    if name.contains('/') {
        return Ok(fs::read_to_string(name)?);
    }

    let custom = templates_dir(config_dir).join(format!("{name}.txt"));
    if custom.exists() {
        return Ok(fs::read_to_string(custom)?);
    }

    let builtin = builtins()
        .iter()
        .find(|(builtin_name, _)| *builtin_name == name)
        .map(|(_, contents)| (*contents).to_string());
    builtin.ok_or_else(|| format!("unknown template: {name}").into())
}

fn parse_and_render(template_text: &str, prompt: &str) -> RenderedTemplate {
    if template_text.contains(SECTION_SYSTEM) || template_text.contains(SECTION_USER) {
        let (system, user) = split_sections(template_text);
        return RenderedTemplate {
            system: render_placeholder(system.trim(), prompt),
            user: render_placeholder(user.trim(), prompt),
        };
    }

    if template_text.contains("{{prompt}}") {
        return RenderedTemplate {
            system: String::new(),
            user: render_placeholder(template_text.trim(), prompt),
        };
    }

    RenderedTemplate {
        system: template_text.trim().to_string(),
        user: prompt.trim().to_string(),
    }
}

fn split_sections(template_text: &str) -> (&str, &str) {
    let system_start = template_text.find(SECTION_SYSTEM);
    let user_start = template_text.find(SECTION_USER);

    match (system_start, user_start) {
        (Some(system_idx), Some(user_idx)) if system_idx < user_idx => {
            let system = &template_text[system_idx + SECTION_SYSTEM.len()..user_idx];
            let user = &template_text[user_idx + SECTION_USER.len()..];
            (system, user)
        }
        (Some(system_idx), None) => (&template_text[system_idx + SECTION_SYSTEM.len()..], ""),
        (None, Some(user_idx)) => ("", &template_text[user_idx + SECTION_USER.len()..]),
        _ => ("", template_text),
    }
}

fn render_placeholder(template_text: &str, prompt: &str) -> String {
    template_text.replace("{{prompt}}", prompt)
}

fn builtins() -> [(&'static str, &'static str); 3] {
    [
        ("default", DEFAULT_TEMPLATE),
        ("code-review", REVIEW_TEMPLATE),
        ("commit-message", COMMIT_TEMPLATE),
    ]
}

#[cfg(test)]
mod tests {
    use super::{parse_and_render, RenderedTemplate};

    #[test]
    fn renders_inline_prompt_template() {
        let rendered = parse_and_render("prefix {{prompt}} suffix", "hello");
        assert_eq!(rendered.user, "prefix hello suffix");
        assert!(rendered.system.is_empty());
    }

    #[test]
    fn renders_sectioned_template() {
        let rendered = parse_and_render(
            "--- system ---\nbe terse\n--- user ---\nfix {{prompt}}\n",
            "this",
        );
        assert_eq!(
            rendered,
            RenderedTemplate {
                system: "be terse".to_string(),
                user: "fix this".to_string(),
            }
        );
    }

    #[test]
    fn treats_plain_text_as_system_prompt() {
        let rendered = parse_and_render("You are concise.", "hello");
        assert_eq!(rendered.system, "You are concise.");
        assert_eq!(rendered.user, "hello");
    }
}
