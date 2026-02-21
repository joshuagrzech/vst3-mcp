use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const PLUGIN_DOCS_ENV: &str = "AGENTAUDIO_PLUGIN_DOCS_DIR";
const SOUND_GUIDE_ENV: &str = "AGENTAUDIO_SOUND_DESIGN_DIR";
const DEFAULT_PLUGIN_DOCS_DIR: &str = "docs/plugins";
const DEFAULT_SOUND_GUIDE_DIR: &str = "docs/sound-design";
const MAX_EXCERPTS: usize = 3;
const MAX_EXCERPT_CHARS: usize = 550;

const STOPWORDS: [&str; 31] = [
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "into", "is",
    "it", "of", "on", "or", "that", "the", "their", "there", "these", "this", "to", "use",
    "using", "what", "with", "you", "your",
];

#[derive(Debug, Clone)]
struct ScoredExcerpt {
    source: String,
    score: usize,
    text: String,
}

pub fn search_plugin_docs(plugin_name: &str, query: &str) -> Result<serde_json::Value, String> {
    let plugin_name = plugin_name.trim();
    if plugin_name.is_empty() {
        return Err("plugin_name is required.".to_string());
    }

    let query = query.trim();
    if query.is_empty() {
        return Err("query is required.".to_string());
    }

    let docs_dir = docs_dir_from_env(PLUGIN_DOCS_ENV, DEFAULT_PLUGIN_DOCS_DIR);
    let files = collect_doc_files(&docs_dir)?;
    let candidate_files = select_plugin_doc_files(&files, plugin_name);

    if candidate_files.is_empty() {
        return Err(format!(
            "No plugin docs matched '{plugin_name}' in '{}'. Add a file such as '{}.md' (or set {}) and try again. You can also use search_params/find_vst_parameter for direct parameter exploration.",
            docs_dir.display(),
            plugin_name,
            PLUGIN_DOCS_ENV
        ));
    }

    let terms = tokenize_keywords(query);
    let scored = score_files(&candidate_files, &terms)?;
    if scored.is_empty() {
        return Err(format!(
            "No relevant excerpts found for plugin '{plugin_name}' and query '{query}'. Try a narrower query (for example a specific control name), or use search_params/find_vst_parameter."
        ));
    }

    let top = top_excerpts(scored);
    let excerpts: Vec<String> = top.iter().map(|e| e.text.clone()).collect();
    let sources: Vec<String> = top.iter().map(|e| e.source.clone()).collect();

    Ok(serde_json::json!({
        "plugin_name": plugin_name,
        "query": query,
        "excerpts": excerpts,
        "sources": sources,
    }))
}

pub fn search_sound_design_guide(
    topic: &str,
    query: Option<&str>,
) -> Result<serde_json::Value, String> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err("topic is required.".to_string());
    }

    let query = query.map(str::trim).filter(|q| !q.is_empty());
    let docs_dir = docs_dir_from_env(SOUND_GUIDE_ENV, DEFAULT_SOUND_GUIDE_DIR);
    let files = collect_doc_files(&docs_dir)?;

    let search_query = if let Some(q) = query {
        format!("{topic} {q}")
    } else {
        topic.to_string()
    };
    let terms = tokenize_keywords(&search_query);
    let scored = score_files(&files, &terms)?;
    if scored.is_empty() {
        return Err(format!(
            "No sound design guide excerpts matched topic '{topic}'. Try a different topic/query, or fall back to search_params/find_vst_parameter to explore plugin controls directly."
        ));
    }

    let mut grouped: HashMap<String, Vec<ScoredExcerpt>> = HashMap::new();
    for excerpt in scored {
        grouped
            .entry(excerpt.source.clone())
            .or_default()
            .push(excerpt);
    }

    let mut ranked_guides: Vec<(String, usize, Vec<ScoredExcerpt>)> = grouped
        .into_iter()
        .map(|(source, mut excerpts)| {
            excerpts.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
            let score = excerpts.iter().take(MAX_EXCERPTS).map(|e| e.score).sum();
            (source, score, excerpts)
        })
        .collect();
    ranked_guides.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let (guide, _, excerpts) = ranked_guides
        .into_iter()
        .next()
        .ok_or_else(|| "No matching sound design guide found.".to_string())?;
    let top = top_excerpts(excerpts);
    let excerpt_texts: Vec<String> = top.iter().map(|e| e.text.clone()).collect();

    Ok(serde_json::json!({
        "topic": topic,
        "query": query,
        "guide": guide,
        "excerpts": excerpt_texts,
        "step_by_step_recipe": excerpt_texts.join("\n\n"),
    }))
}

fn docs_dir_from_env(env_name: &str, default_relative: &str) -> PathBuf {
    if let Ok(raw) = std::env::var(env_name) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join(default_relative)
}

fn collect_doc_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Err(format!(
            "Documentation directory '{}' does not exist. Create it (or set AGENTAUDIO_PLUGIN_DOCS_DIR / AGENTAUDIO_SOUND_DESIGN_DIR) and add Markdown files.",
            dir.display()
        ));
    }

    let mut files = Vec::new();
    collect_doc_files_recursive(dir, &mut files)
        .map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))?;
    files.sort();

    if files.is_empty() {
        return Err(format!(
            "No documentation files found in '{}'. Add .md/.txt/.json files first.",
            dir.display()
        ));
    }

    Ok(files)
}

fn collect_doc_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry_result in fs::read_dir(dir)? {
        let entry = entry_result?;
        let path = entry.path();
        if path.is_dir() {
            collect_doc_files_recursive(&path, out)?;
            continue;
        }
        if is_supported_doc_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_supported_doc_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase()),
        Some(ext) if ext == "md" || ext == "markdown" || ext == "txt" || ext == "json"
    )
}

fn select_plugin_doc_files(files: &[PathBuf], plugin_name: &str) -> Vec<PathBuf> {
    let plugin_key = normalize_ascii_alnum(plugin_name);
    if plugin_key.is_empty() {
        return Vec::new();
    }

    let plugin_terms = tokenize_keywords(plugin_name);
    files
        .iter()
        .filter(|path| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let stem_key = normalize_ascii_alnum(stem);
            if stem_key.contains(&plugin_key) || plugin_key.contains(&stem_key) {
                return true;
            }

            let stem_lower = stem.to_lowercase();
            plugin_terms.iter().all(|term| stem_lower.contains(term))
        })
        .cloned()
        .collect()
}

fn score_files(files: &[PathBuf], terms: &[String]) -> Result<Vec<ScoredExcerpt>, String> {
    let mut scored = Vec::new();
    for file in files {
        let text = read_doc_text(file)?;
        let source = relative_display_path(file);
        for chunk in split_into_chunks(&text) {
            let score = score_chunk(&chunk, terms);
            if score == 0 {
                continue;
            }
            scored.push(ScoredExcerpt {
                source: source.clone(),
                score,
                text: truncate_excerpt(&chunk, MAX_EXCERPT_CHARS),
            });
        }
    }

    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.text.cmp(&b.text))
    });
    scored.dedup_by(|a, b| a.source == b.source && a.text == b.text);
    Ok(scored)
}

fn read_doc_text(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
    if raw.trim().is_empty() {
        return Ok(String::new());
    }

    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
        return serde_json::to_string_pretty(&value)
            .map_err(|e| format!("Failed to format JSON '{}': {}", path.display(), e));
    }

    Ok(raw)
}

fn split_into_chunks(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let mut chunks: Vec<String> = normalized
        .split("\n\n")
        .map(clean_chunk)
        .filter(|chunk| !chunk.is_empty())
        .collect();

    if chunks.is_empty() {
        let compact = clean_chunk(&normalized);
        if !compact.is_empty() {
            chunks.push(compact);
        }
    }

    chunks
}

fn clean_chunk(chunk: &str) -> String {
    chunk
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize_keywords(input: &str) -> Vec<String> {
    let stopwords: HashSet<&'static str> = STOPWORDS.into_iter().collect();
    let mut terms: Vec<String> = input
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .filter(|token| token.len() >= 2)
        .filter(|token| !stopwords.contains(*token))
        .map(ToString::to_string)
        .collect();

    terms.sort();
    terms.dedup();
    terms
}

fn score_chunk(chunk: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }

    let lower = chunk.to_lowercase();
    let mut unique_hits = 0usize;
    let mut total_hits = 0usize;
    for term in terms {
        let hits = count_term_matches(&lower, term);
        if hits > 0 {
            unique_hits += 1;
            total_hits += hits;
        }
    }

    unique_hits * 20 + total_hits
}

fn count_term_matches(haystack: &str, term: &str) -> usize {
    if term.len() <= 2 {
        haystack
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|token| *token == term)
            .count()
    } else {
        haystack.matches(term).count()
    }
}

fn top_excerpts(mut scored: Vec<ScoredExcerpt>) -> Vec<ScoredExcerpt> {
    if scored.len() > MAX_EXCERPTS {
        scored.truncate(MAX_EXCERPTS);
    }
    scored
}

fn truncate_excerpt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut idx = 0usize;
    let mut char_count = 0usize;
    for (i, _) in text.char_indices() {
        if char_count == max_chars {
            break;
        }
        idx = i;
        char_count += 1;
    }

    let mut truncated = text[..=idx].to_string();
    if let Some(last_space) = truncated.rfind(' ') {
        truncated.truncate(last_space);
    }
    truncated.push_str("...");
    truncated
}

fn normalize_ascii_alnum(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn relative_display_path(path: &Path) -> String {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    path.strip_prefix(manifest_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_keywords_removes_stopwords_and_duplicates() {
        let terms = tokenize_keywords("How to set the vocal compression and compression ratio");
        assert_eq!(
            terms,
            vec![
                "compression".to_string(),
                "ratio".to_string(),
                "set".to_string(),
                "vocal".to_string()
            ]
        );
    }

    #[test]
    fn score_chunk_prefers_more_term_hits() {
        let terms = vec!["lfo".to_string(), "routing".to_string()];
        let high = score_chunk("LFO routing matrix with LFO 1 -> cutoff", &terms);
        let low = score_chunk("Routing controls are available", &terms);
        assert!(high > low);
    }

    #[test]
    fn truncate_excerpt_adds_ellipsis_for_long_text() {
        let text = "one two three four five six seven eight nine ten";
        let out = truncate_excerpt(text, 12);
        assert!(out.ends_with("..."));
        assert!(out.len() < text.len());
    }
}
