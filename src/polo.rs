//! Ham2K PoLo callsign notes file support.
//!
//! Parses callsign notes files and manages URL fetching with background refresh.

/// Parse Ham2K PoLo notes file content into a list of callsigns.
///
/// File format:
/// - One callsign per line, followed by optional notes
/// - Lines starting with # are comments
/// - Empty lines are ignored
pub fn parse_polo_notes(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            // Extract callsign (first whitespace-delimited token)
            trimmed.split_whitespace().next().map(|s| s.to_uppercase())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_polo_notes_basic() {
        let content = "VK1AO Alan\nVK4KC Marty\nKI2D Sebastián";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["VK1AO", "VK4KC", "KI2D"]);
    }

    #[test]
    fn test_parse_polo_notes_with_comments() {
        let content = "# My watchlist\nW6JSV Jay\n# Another comment\nK6ABC Bob";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_empty_lines() {
        let content = "W6JSV Jay\n\n\nK6ABC Bob\n";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_whitespace() {
        let content = "  W6JSV Jay\n\t\nK6ABC Bob  ";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_empty() {
        let content = "";
        let callsigns = parse_polo_notes(content);
        assert!(callsigns.is_empty());
    }
}
