//! Bench report: one HTML page with blind pairs. The data is embedded into the template as JSON.
use serde::{Deserialize, Serialize};

/// `engine` is the local speech model's folder name (`parakeet-ultra-int8` is the app's model); `latency_ms` is its
/// recognition time and `cost_usd` is 0 — runs from before the local models keep their cloud prices. `error` is the
/// engine error text (no dictation), and then `text` is empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SttRun {
    pub engine: String,
    pub text: String,
    pub latency_ms: u64,
    pub cost_usd: f64,
    pub error: Option<String>,
}

/// `outcome` is the `RefineOutcome` name (`Refined`, `RawFallback`, `RawAgent(SignedOut)`, …); if it is not `Refined`, `text`
/// holds the raw text, exactly what the app would have pasted. Tokens relate latency to the length of the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefineRun {
    pub model: String,
    pub text: String,
    pub outcome: String,
    pub latency_ms: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
}

/// One recording: `file` is the absolute path to the wav (the report plays it via `file:///`), `app` is the
/// recording's app, or `UNKNOWN_APP` if it was not recognized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub file: String,
    pub app: String,
    pub audio_ms: u64,
    pub stt: Vec<SttRun>,
    pub refine: Vec<RefineRun>,
}

const TEMPLATE: &str = include_str!("report.html");

/// Template + data. `</` is escaped so dictation text cannot close the <script> tag.
/// `<!--` too: from it, HTML parsing inside <script> enters a mode where a real `</script>` no longer closes the tag.
/// The data goes in as a JS literal: `<\/` and `<\!` in a JS string are the same `</` and `<!`.
pub fn embed(items: &[Item]) -> String {
    let json = serde_json::to_string(items).expect("bench data").replace("</", r"<\/").replace("<!--", r"<\!--");
    TEMPLATE.replacen("/*DATA*/", &json, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_is_embedded_and_script_safe() {
        let items = vec![Item {
            file: "C:/x/a.wav".into(),
            app: "Telegram Desktop".into(),
            audio_ms: 1_000,
            stt: vec![],
            refine: vec![RefineRun { model: "claude-haiku-5-5".into(), text: "</script><b>".into(), outcome: "Refined".into(), latency_ms: 1, cost_usd: 0.0, input_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, output_tokens: 0 }],
        }];
        let html = embed(&items);
        assert!(!html.contains("/*DATA*/"));
        assert!(html.contains("claude-haiku-5-5"));
        assert!(!html.contains("</script><b>"));
        assert!(html.contains(r"<\/script><b>"));
    }

    fn item(stt_text: &str, refine_text: &str) -> Item {
        Item {
            file: r"C:\Users\Тест Тестов\AppData\Local\CUE\bench\recordings\2026-10-27T10-00-00Z-1.wav".into(),
            app: "Notepad".into(),
            audio_ms: 12_345,
            stt: vec![SttRun { engine: "parakeet-ultra-int8".into(), text: stt_text.into(), latency_ms: 300, cost_usd: 0.0, error: None }],
            refine: vec![RefineRun {
                model: "claude-haiku-5-5".into(),
                text: refine_text.into(),
                outcome: "Refined".into(),
                latency_ms: 1_200,
                cost_usd: 0.00012,
                input_tokens: 120,
                cache_read_tokens: 950,
                cache_write_tokens: 0,
                output_tokens: 40,
            }],
        }
    }

    /// The data sits between `const DATA = ` and the `;` at the end of the same template line (line breaks in the data are escaped).
    fn embedded(html: &str) -> &str {
        let start = html.find("const DATA = ").expect("data line") + "const DATA = ".len();
        let line = html[start..].lines().next().expect("data line");
        line.strip_suffix(';').expect("`;` at the end of the data line")
    }

    #[test]
    fn comment_opener_cannot_swallow_the_closing_script_tag() {
        let html = embed(&[item("<!-- <script>", "</SCRIPT>")]);
        let data = embedded(&html);
        assert!(!data.contains("<!--") && !data.contains("</"), "{data}");
        // There is no other `</script>` in the file: only the template's real end of script closes the tag.
        assert_eq!(html.to_lowercase().matches("</script>").count(), 1);
    }

    #[test]
    fn embedded_data_reads_back_unchanged() {
        let items = vec![item("ну вот «кавычки» и \"ещё\"\nи </b>", "Вот «кавычки» и \"ещё\".\n</b>"), item("", "")];
        let html = embed(&items);
        // `<\/` is a valid JSON escape: data without `<!--` reads back as is.
        let back: Vec<Item> = serde_json::from_str(embedded(&html)).unwrap();
        assert_eq!(back, items);
        assert_eq!(html.matches("const DATA = ").count(), 1);
    }
}
