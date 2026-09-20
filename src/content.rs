use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use ammonia::Builder;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use walkdir::WalkDir;

use crate::app::{MAX_FILE_BYTES, MAX_IMPORT_FILES};

pub const ALLOWED_ASSET_EXTENSIONS: &[&str] =
    &["png", "jpg", "jpeg", "webp", "gif", "svg", "mp4", "webm"];

#[derive(Clone, Debug, Serialize)]
pub struct Article {
    pub slug: String,
    pub title: String,
    pub subtitle: String,
    pub hero: Option<String>,
    #[serde(skip)]
    pub published_at: DateTime<FixedOffset>,
    #[serde(skip)]
    pub release_at: DateTime<FixedOffset>,
    pub body: String,
    pub draft: bool,
    pub url: String,
    pub hero_url: Option<String>,
    pub publication_date: String,
    pub release_date: String,
    pub date_short: String,
    pub date_long: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArticlePayload {
    pub slug: String,
    pub title: String,
    pub subtitle: String,
    pub hero: String,
    pub publication_date: String,
    pub release_date: String,
    pub draft: bool,
    pub published: bool,
    pub body: String,
    pub assets: Vec<String>,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ArticleInput {
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub hero: String,
    pub publication_date: Option<Value>,
    pub release_date: Option<Value>,
    pub draft: Option<bool>,
    pub published: Option<bool>,
    #[serde(default)]
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteHome {
    pub eyebrow: String,
    pub title: String,
    pub empty: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteConfig {
    pub name: String,
    pub wordmark: String,
    pub tagline: String,
    pub language: String,
    pub home: SiteHome,
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            name: "MF-Blog".to_string(),
            wordmark: "MF-Blog".to_string(),
            tagline: "Markdown publishing, organized by folders.".to_string(),
            language: "en".to_string(),
            home: SiteHome {
                eyebrow: "Latest articles".to_string(),
                title: "Writing, published\nfrom Markdown.".to_string(),
                empty: "No articles have been published yet.".to_string(),
            },
        }
    }
}

pub fn load_site_config(path: &Path) -> Result<SiteConfig, String> {
    let mut config = SiteConfig::default();
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(config),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let value: Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("site.yaml is invalid: {error}"))?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| "site.yaml must contain a mapping".to_string())?;

    for (key, target) in [
        ("name", &mut config.name),
        ("wordmark", &mut config.wordmark),
        ("tagline", &mut config.tagline),
        ("language", &mut config.language),
    ] {
        if let Some(value) = mapping.get(Value::String(key.to_string())) {
            let text = value
                .as_str()
                .ok_or_else(|| format!("site.yaml field {key:?} must be a non-empty string"))?
                .trim();
            if text.is_empty() {
                return Err(format!("site.yaml field {key:?} must be a non-empty string"));
            }
            *target = text.to_string();
        }
    }

    if let Some(home) = mapping.get(Value::String("home".to_string())) {
        let home = home
            .as_mapping()
            .ok_or_else(|| "site.yaml field 'home' must be a mapping".to_string())?;
        for (key, target) in [
            ("eyebrow", &mut config.home.eyebrow),
            ("title", &mut config.home.title),
            ("empty", &mut config.home.empty),
        ] {
            if let Some(value) = home.get(Value::String(key.to_string())) {
                *target = value
                    .as_str()
                    .ok_or_else(|| format!("site.yaml home.{key} must be a string"))?
                    .trim()
                    .to_string();
            }
        }
    }

    Ok(config)
}

pub fn split_front_matter(text: &str) -> Result<(Mapping, String), String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("Article must begin with YAML front matter".to_string());
    }

    let mut metadata_lines = Vec::new();
    let mut found_end = false;
    let mut body_lines = Vec::new();

    for line in lines {
        if !found_end && line.trim() == "---" {
            found_end = true;
            continue;
        }
        if found_end {
            body_lines.push(line);
        } else {
            metadata_lines.push(line);
        }
    }

    if !found_end {
        return Err("YAML front matter is not closed".to_string());
    }

    let metadata: Value = serde_yaml::from_str(&metadata_lines.join("\n"))
        .map_err(|error| format!("invalid YAML front matter: {error}"))?;
    let mapping = metadata
        .as_mapping()
        .cloned()
        .ok_or_else(|| "YAML front matter must be a mapping".to_string())?;

    Ok((mapping, body_lines.join("\n").trim_start().to_string()))
}

fn map_get<'a>(mapping: &'a Mapping, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| mapping.get(Value::String((*key).to_string())))
}

fn required_string(mapping: &Mapping, key: &str) -> Result<String, String> {
    let value = map_get(mapping, &[key])
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if value.is_empty() {
        Err(format!("{key} is required"))
    } else {
        Ok(value.to_string())
    }
}

fn optional_string(mapping: &Mapping, key: &str) -> Result<Option<String>, String> {
    match map_get(mapping, &[key]) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.trim().to_string())),
        _ => Err(format!("{key} must be a string")),
    }
}

pub fn parse_datetime(value: Option<&Value>, field: &str, timezone: Tz) -> Result<DateTime<FixedOffset>, String> {
    let value = value.ok_or_else(|| format!("Missing or invalid {field}"))?;
    let text = match value {
        Value::String(text) => text.clone(),
        _ => serde_yaml::to_string(value)
            .map_err(|_| format!("Missing or invalid {field}"))?
            .trim()
            .to_string(),
    };

    if let Ok(parsed) = DateTime::parse_from_rfc3339(&text) {
        return Ok(parsed);
    }
    if let Ok(date) = NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0).ok_or_else(|| format!("Invalid {field}: {text:?}"))?;
        let local = timezone
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| format!("Invalid {field}: {text:?}"))?;
        return Ok(local.fixed_offset());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S") {
        let local = timezone
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| format!("Invalid {field}: {text:?}"))?;
        return Ok(local.fixed_offset());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M") {
        let local = timezone
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| format!("Invalid {field}: {text:?}"))?;
        return Ok(local.fixed_offset());
    }

    Err(format!("Invalid {field}: {text:?}"))
}

pub fn load_article(path: &Path, timezone: Tz) -> Result<Article, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let (metadata, body) = split_front_matter(&source)?;
    let title = required_string(&metadata, "title")?;
    let subtitle = optional_string(&metadata, "subtitle")?.unwrap_or_default();
    let hero = optional_string(&metadata, "hero")?.filter(|value| !value.is_empty());
    let published_value = map_get(&metadata, &["publication_date", "publishedAt"]);
    let release_value = map_get(&metadata, &["release_date", "releaseAt"]).or(published_value);
    let published_at = parse_datetime(published_value, "publication_date", timezone)?;
    let release_at = parse_datetime(release_value, "release_date", timezone)?;
    let draft = match map_get(&metadata, &["draft"]) {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("draft must be true or false, without quotes".to_string()),
    };
    let slug = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| "article path has no valid slug".to_string())?
        .to_string();

    Ok(Article {
        url: format!("/articles/{slug}/"),
        hero_url: hero.as_deref().map(|value| asset_url(&slug, value)),
        publication_date: published_at.to_rfc3339(),
        release_date: release_at.to_rfc3339(),
        date_short: published_at.format("%d %b %Y").to_string(),
        date_long: published_at.format("%d %B %Y").to_string(),
        slug,
        title,
        subtitle,
        hero,
        published_at,
        release_at,
        body,
        draft,
    })
}

pub fn discover_all_articles(root: &Path, timezone: Tz) -> (Vec<Article>, Vec<String>) {
    let mut articles = Vec::new();
    let mut errors = Vec::new();

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (articles, errors),
        Err(error) => {
            errors.push(format!("{}: {error}", root.display()));
            return (articles, errors);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path().join("article.md");
        if !path.is_file() {
            continue;
        }
        match load_article(&path, timezone) {
            Ok(article) => articles.push(article),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    articles.sort_by(|left, right| right.published_at.cmp(&left.published_at));
    (articles, errors)
}

pub fn discover_articles(root: &Path, timezone: Tz, now: DateTime<Utc>) -> (Vec<Article>, Vec<String>) {
    let (articles, errors) = discover_all_articles(root, timezone);
    (
        articles
            .into_iter()
            .filter(|article| !article.draft && article.release_at.with_timezone(&Utc) <= now)
            .collect(),
        errors,
    )
}

pub fn validate_slug(value: &str) -> Result<String, String> {
    let slug = value.trim().to_ascii_lowercase();
    let valid = Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
        .expect("slug regex is valid")
        .is_match(&slug);
    valid
        .then_some(slug)
        .ok_or_else(|| "slug must contain lowercase letters, numbers, and single hyphens".to_string())
}

pub fn asset_url(slug: &str, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('#') || trimmed.contains("://") || trimmed.starts_with("//") {
        return trimmed.to_string();
    }

    let normalized = trimmed.replace('\\', "/");
    let mut parts: Vec<&str> = normalized.split('/').collect();
    if parts.first() == Some(&".") {
        parts.remove(0);
    }
    if parts.first() == Some(&"assets") {
        parts.remove(0);
    }
    if parts.is_empty() || parts.iter().any(|part| part.is_empty() || *part == "." || *part == "..") {
        return "#".to_string();
    }

    format!("/articles/{slug}/assets/{}", parts.join("/"))
}

pub fn render_markdown(article: &Article) -> String {
    let parser = Parser::new_ext(&article.body, Options::all());
    let mut raw_html = String::new();
    html::push_html(&mut raw_html, parser);

    let image_re = Regex::new(r#"(<img\b[^>]*\bsrc=")([^"]+)(")"#).unwrap();
    let rewritten = image_re.replace_all(&raw_html, |caps: &regex::Captures<'_>| {
        let url = asset_url(&article.slug, &caps[2]);
        format!("{}{}{}", &caps[1], url, &caps[3])
    });
    let link_re = Regex::new(r#"(<a\b[^>]*\bhref=")((?:\./)?assets/[^"]+)(")"#).unwrap();
    let rewritten = link_re.replace_all(&rewritten, |caps: &regex::Captures<'_>| {
        let url = asset_url(&article.slug, &caps[2]);
        format!("{}{}{}", &caps[1], url, &caps[3])
    });

    let mut builder = Builder::default();
    builder
        .add_tags([
            "h1", "h2", "h3", "h4", "h5", "h6", "p", "pre", "code", "hr", "br", "img",
            "table", "thead", "tbody", "tr", "th", "td", "del", "blockquote",
        ])
        .add_tag_attributes("img", ["src", "alt", "title", "loading"])
        .add_tag_attributes("code", ["class"])
        .add_tag_attributes("a", ["href", "title"]);
    builder.clean(&rewritten).to_string()
}

pub fn article_payload(article: &Article, article_root: &Path) -> ArticlePayload {
    let assets_root = article_root.join(&article.slug).join("assets");
    let mut assets = Vec::new();
    if assets_root.is_dir() {
        for entry in WalkDir::new(&assets_root).into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Ok(relative) = entry.path().strip_prefix(&assets_root) {
                    assets.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    assets.sort();

    ArticlePayload {
        slug: article.slug.clone(),
        title: article.title.clone(),
        subtitle: article.subtitle.clone(),
        hero: article.hero.clone().unwrap_or_default(),
        publication_date: article.publication_date.clone(),
        release_date: article.release_date.clone(),
        draft: article.draft,
        published: !article.draft,
        body: article.body.clone(),
        assets,
        url: article.url.clone(),
    }
}

fn value_to_datetime(value: Option<&Value>, field: &str, timezone: Tz) -> Result<String, String> {
    parse_datetime(value, field, timezone).map(|value| value.to_rfc3339())
}

pub fn build_article_source(input: &ArticleInput, timezone: Tz) -> Result<String, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("title is required".to_string());
    }
    let hero = input.hero.trim();
    if !hero.is_empty() && asset_url("check", hero) == "#" {
        return Err("hero path is invalid".to_string());
    }

    let draft = input.draft.unwrap_or_else(|| !input.published.unwrap_or(true));
    let publication_date = value_to_datetime(input.publication_date.as_ref(), "publication_date", timezone)?;
    let release_date = value_to_datetime(input.release_date.as_ref(), "release_date", timezone)?;

    let mut metadata = BTreeMap::new();
    metadata.insert("title", Value::String(title.to_string()));
    metadata.insert("subtitle", Value::String(input.subtitle.trim().to_string()));
    metadata.insert(
        "hero",
        if hero.is_empty() { Value::Null } else { Value::String(hero.to_string()) },
    );
    metadata.insert("publication_date", Value::String(publication_date));
    metadata.insert("release_date", Value::String(release_date));
    metadata.insert("draft", Value::Bool(draft));

    let front = serde_yaml::to_string(&metadata)
        .map_err(|error| format!("cannot serialize article metadata: {error}"))?;
    Ok(format!("---\n{}---\n\n{}\n", front.trim_start_matches("---\n").trim_end(), input.body.trim_start()))
}

pub fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("unsafe path: {value}"));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::CurDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(format!("unsafe path: {value}"));
        }
    }
    Ok(path.to_path_buf())
}

pub fn asset_extension_allowed(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| ALLOWED_ASSET_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn validate_asset_file(path: &Path) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("svg")) != Some(true) {
        return Ok(());
    }

    let content = fs::read_to_string(path)
        .map_err(|error| format!("invalid SVG {}: {error}", path.display()))?;
    let document = roxmltree::Document::parse(&content)
        .map_err(|error| format!("invalid SVG {}: {error}", path.display()))?;

    for node in document.descendants().filter(|node| node.is_element()) {
        let tag = node.tag_name().name().to_ascii_lowercase();
        if ["script", "foreignobject", "iframe", "object", "embed", "style"].contains(&tag.as_str()) {
            return Err(format!("active SVG content is not allowed: {}", path.display()));
        }
        for attribute in node.attributes() {
            let name = attribute.name().to_ascii_lowercase();
            let value = attribute.value();
            if name.starts_with("on") || name == "style" {
                return Err(format!("active SVG attributes are not allowed: {}", path.display()));
            }
            if (name == "href" || name.ends_with(":href"))
                && (value.contains("://") || value.starts_with("//"))
            {
                return Err(format!("external SVG references are not allowed: {}", path.display()));
            }
        }
    }

    Ok(())
}

pub fn validate_import_tree(root: &Path, timezone: Tz) -> Result<PathBuf, String> {
    let mut article_files = Vec::new();
    let mut file_count = 0usize;

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| format!("cannot inspect import: {error}"))?;
        if entry.file_type().is_symlink() {
            return Err("symlinks are not allowed".to_string());
        }
        if entry.file_type().is_file() {
            file_count += 1;
            if file_count > MAX_IMPORT_FILES {
                return Err("import contains too many files".to_string());
            }
            if entry.file_name() == "article.md" {
                article_files.push(entry.path().to_path_buf());
            }
        }
    }

    if article_files.len() != 1 {
        return Err("import must contain exactly one article.md".to_string());
    }

    let article_dir = article_files[0]
        .parent()
        .ok_or_else(|| "article.md has no parent folder".to_string())?
        .to_path_buf();

    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.starts_with(&article_dir) {
            let relative = path.strip_prefix(root).unwrap_or(path);
            return Err(format!("file exists outside the article folder: {}", relative.display()));
        }
        let relative = path.strip_prefix(&article_dir).unwrap_or(path);
        if relative == Path::new("article.md") {
            continue;
        }
        let valid_asset = relative.components().next().map(|component| component.as_os_str() == "assets").unwrap_or(false)
            && asset_extension_allowed(path);
        if !valid_asset {
            return Err(format!("file is not allowed: {}", relative.display()));
        }
        let size = fs::metadata(path)
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?
            .len() as usize;
        if size > MAX_FILE_BYTES {
            return Err(format!("file is too large: {}", relative.display()));
        }
        validate_asset_file(path)?;
    }

    load_article(&article_files[0], timezone)?;
    Ok(article_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_are_scoped() {
        assert_eq!(asset_url("hello", "assets/image.png"), "/articles/hello/assets/image.png");
        assert_eq!(asset_url("hello", "../secret.txt"), "#");
    }

    #[test]
    fn quoted_draft_boolean_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("article.md");
        fs::write(
            &file,
            "---\ntitle: Ambiguous\npublication_date: 2020-01-01\ndraft: 'false'\n---\n",
        )
        .unwrap();
        let error = load_article(&file, chrono_tz::Asia::Makassar).unwrap_err();
        assert!(error.contains("draft must be true or false"));
    }

    #[test]
    fn normal_relative_links_survive_markdown_rendering() {
        let published = DateTime::parse_from_rfc3339("2020-01-01T00:00:00+08:00").unwrap();
        let article = Article {
            slug: "links".into(),
            title: "Links".into(),
            subtitle: String::new(),
            hero: None,
            published_at: published,
            release_at: published,
            body: "[Notes](notes)\n\n![Diagram](assets/diagram.png)".into(),
            draft: false,
            url: "/articles/links/".into(),
            hero_url: None,
            publication_date: published.to_rfc3339(),
            release_date: published.to_rfc3339(),
            date_short: String::new(),
            date_long: String::new(),
        };
        let html = render_markdown(&article);
        assert!(html.contains("href=\"notes\""));
        assert!(html.contains("src=\"/articles/links/assets/diagram.png\""));
    }
}
