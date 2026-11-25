//! `browser.page_hint` ([Agent Browser Protocol] §7.2): whether the page the
//! user looks at is one to read or one to do something on — what the app
//! offers first when the user asks for a reflow. No model: the page is
//! counted ([`PageFacts`], by `hint_page.js`) and the counts are judged
//! ([`classify`]).
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::groups::DEFAULT_GROUP;
use super::{BrowserError, BrowserModule};

const PAGE_SCRIPT: &str = include_str!("hint_page.js");
/// A page that shows a dialog does not answer.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
const FORM_FIELDS: u64 = 3;
const READING_CHARS: u64 = 1500;

/// What `hint_page.js` counts in a page.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PageFacts {
    pub url: String,
    pub title: String,
    /// Password fields that are visible.
    pub password_fields: u64,
    /// Visible `input` (but hidden, submit, button, search), `select` and `textarea` elements.
    pub form_fields: u64,
    pub article_elements: u64,
    /// `og:type` is `article`.
    pub og_article: bool,
    /// An `application/ld+json` block describes an Article, NewsArticle or BlogPosting.
    pub ld_article: bool,
    /// Characters of the page's text, white space aside.
    pub text_chars: u64,
    /// Those of them in links.
    pub link_chars: u64,
}

/// `task | reading | unknown`: the first that holds.
pub fn classify(facts: &PageFacts) -> &'static str {
    if facts.password_fields > 0 || facts.form_fields >= FORM_FIELDS {
        "task"
    } else if facts.article_elements > 0 || facts.og_article || facts.ld_article || (facts.text_chars >= READING_CHARS && facts.link_chars * 2 < facts.text_chars) {
        "reading"
    } else {
        "unknown"
    }
}

impl BrowserModule {
    /// `browser.page_hint`: one evaluation in the current tab. Starts no
    /// browser and takes no lease; a page that does not answer is `unknown`.
    pub async fn page_hint(&self) -> Result<Value, BrowserError> {
        self.usable()?;
        let Some(tab) = self.active_tab(DEFAULT_GROUP) else {
            return Ok(json!({ "hint": "unknown", "url": null, "title": null }));
        };
        let facts = match (self.cdp(), &tab.session) {
            (Some(cdp), Some(session)) => {
                let asked = json!({ "expression": PAGE_SCRIPT, "returnByValue": true });
                let answer = cdp.send_within(Some(session), "Runtime.evaluate", asked, ANSWER_WAIT).await;
                answer.ok().and_then(|answer| serde_json::from_value::<PageFacts>(answer["result"]["value"].clone()).ok())
            }
            _ => None,
        };
        Ok(match facts {
            Some(facts) => json!({ "hint": classify(&facts), "url": facts.url, "title": facts.title }),
            None => json!({ "hint": "unknown", "url": tab.url, "title": tab.title }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the script counts in pages of each kind.
    #[test]
    fn a_page_is_a_task_a_reading_or_unknown_by_what_it_has() {
        let page = PageFacts::default;
        let cases: [(&str, PageFacts, &str); 14] = [
            ("an empty page", page(), "unknown"),
            ("a sign-in page", PageFacts { password_fields: 1, form_fields: 2, ..page() }, "task"),
            ("a password field in an article", PageFacts { password_fields: 1, form_fields: 1, article_elements: 1, text_chars: 9000, ..page() }, "task"),
            ("a form of three fields", PageFacts { form_fields: 3, text_chars: 400, ..page() }, "task"),
            ("an order form in a long page", PageFacts { form_fields: 7, text_chars: 6000, link_chars: 200, og_article: true, ..page() }, "task"),
            ("an article with a comment box and a newsletter field", PageFacts { form_fields: 2, article_elements: 1, text_chars: 5000, link_chars: 300, ..page() }, "reading"),
            ("an article element, hardly any text yet", PageFacts { article_elements: 1, text_chars: 80, ..page() }, "reading"),
            ("og:type article", PageFacts { og_article: true, ..page() }, "reading"),
            ("a BlogPosting in ld+json", PageFacts { ld_article: true, text_chars: 300, link_chars: 290, ..page() }, "reading"),
            ("a long text with few links", PageFacts { text_chars: 1500, link_chars: 749, ..page() }, "reading"),
            ("a long text, half of it links", PageFacts { text_chars: 1500, link_chars: 750, ..page() }, "unknown"),
            ("a portal of links", PageFacts { text_chars: 12000, link_chars: 9500, form_fields: 1, ..page() }, "unknown"),
            ("a text a little too short", PageFacts { text_chars: 1499, ..page() }, "unknown"),
            ("a search box and a results list", PageFacts { form_fields: 1, text_chars: 3000, link_chars: 2000, ..page() }, "unknown"),
        ];
        for (what, facts, hint) in cases {
            assert_eq!(classify(&facts), hint, "{what}");
        }
        // What the page script returns is what is judged.
        let counted = json!({
            "url": "https://example.com/login?next=%2F", "title": "Sign in",
            "password_fields": 1, "form_fields": 2, "article_elements": 0, "og_article": false, "ld_article": false, "text_chars": 120, "link_chars": 30,
        });
        let facts: PageFacts = serde_json::from_value(counted).unwrap();
        assert_eq!((facts.url.as_str(), facts.title.as_str(), classify(&facts)), ("https://example.com/login?next=%2F", "Sign in", "task"));
        assert!(serde_json::from_value::<PageFacts>(Value::Null).is_err() && serde_json::from_value::<PageFacts>(json!({})).is_err(), "a page that answered nothing");
    }
}
