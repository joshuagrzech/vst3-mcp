//! Parameter name matching and scoring for find_vst_parameter / get_params_by_name.

/// Returns group prefix for a parameter title (e.g. "Osc 1 Level" -> "Osc 1").
pub fn param_group_prefix(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(3, ' ').collect();
    if parts.len() >= 2 && parts[1].parse::<u32>().is_ok() {
        format!("{} {}", parts[0], parts[1])
    } else {
        parts.first().map(|s| (*s).to_string()).unwrap_or_default()
    }
}

/// Returns (primary_terms, alias_terms) for scoring.
pub fn query_terms_for_scoring(query: &str) -> (Vec<String>, Vec<String>) {
    let lower = query.to_lowercase();
    let primary: Vec<String> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
        .collect();

    let mut aliases: Vec<String> = Vec::new();
    if lower.contains("brighter") {
        aliases.extend(
            ["bright", "brightness", "treble", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("harsh") {
        aliases.extend(
            ["harsh", "resonance", "q", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("reverb") {
        aliases.extend(["room", "wet"].iter().map(|s| s.to_string()));
    }

    aliases.retain(|a| !primary.contains(a));
    aliases.sort();
    aliases.dedup();
    (primary, aliases)
}

/// Score a param against primary and alias terms. Returns 0 if no match.
pub fn score_param(param: &serde_json::Value, primary: &[String], aliases: &[String]) -> u32 {
    let name = param
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let name_words: Vec<&str> = name.split_whitespace().collect();
    let display = param
        .get("display")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let mut score = 0u32;

    for term in primary {
        if name == *term {
            score += 2000;
        } else if name_words.iter().any(|w| *w == term.as_str()) {
            score += 100;
        } else if name.starts_with(term.as_str()) {
            score += 80;
        } else if name.contains(term.as_str()) {
            score += 40;
        }
        if display.contains(term.as_str()) {
            score += 5;
        }
    }
    for term in aliases {
        if name_words.iter().any(|w| *w == term.as_str()) {
            score += 10;
        } else if name.contains(term.as_str()) {
            score += 3;
        }
    }
    score
}
