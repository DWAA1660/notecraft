//! Plain text search across the pages of a notebooks folder.

use crate::page::load_page;
use crate::tree::{child_dirs, page_file, read_node};
use notecraft_formats::{boxes, html, render};
use notecraft_model::PageFormat;
use std::path::{Path, PathBuf};

/// A real tag on a page, with its source identity rather than a match in raw source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagHit {
    pub path: PathBuf,
    pub title: String,
    pub section_path: Option<PathBuf>,
    pub section: String,
    pub notebook_path: PathBuf,
    pub notebook: String,
    pub block: usize,
    pub tag: notecraft_model::Tag,
    pub text: String,
    pub done: Option<bool>,
}

impl TagHit {
    /// The source can change while the summary is open. Do not focus another note at its old index.
    pub fn matches_block(&self, block: &notecraft_formats::Block) -> bool {
        tags_of_block(block).into_iter().any(|(tag, done, text)| tag == self.tag && done == self.done && text == self.text)
    }
}

fn tags_of_block(content: &notecraft_formats::Block) -> Vec<(notecraft_model::Tag, Option<bool>, String)> {
    use notecraft_formats::Block;
    use notecraft_model::Tag;
    let (spans, task) = match content {
        Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Quote(spans) => (spans, None),
        Block::Item { spans, task, .. } => (spans, *task),
        _ => return Vec::new(),
    };
    let text = spans.iter().map(|s| s.text.as_str()).collect::<String>();
    let prefix = spans.iter().take_while(|s| !s.code && !s.math && s.image.is_none()).map(|s| s.text.as_str()).collect::<String>();
    let (marker, rest) = notecraft_model::Tag::split(&text);
    let marker = marker.filter(|tag| Tag::split(&prefix).0 == Some(*tag));
    let rest = if marker.is_some() { rest } else { &text };
    let mut tags = Vec::new();
    if let Some(done) = task {
        tags.push((Tag::ToDo, Some(done), rest.to_owned()));
    }
    if let Some(tag) = marker
        && !(tag == Tag::ToDo && task.is_some())
    {
        tags.push((tag, None, rest.to_owned()));
    }
    tags
}

/// Read tags from the open notebooks. `within` restricts the result to a real node;
/// `exact_page` excludes sub-pages. A read failure is an error, never an empty summary.
pub fn find_tags(notebooks: &[crate::tree::Node], within: Option<&Path>, exact_page: bool, incomplete: bool) -> crate::Result<Vec<TagHit>> {
    let mut query = TagQuery { within, exact_page, incomplete, seen: Default::default(), hits: Vec::new() };
    for notebook in notebooks {
        // Session's open-notebook paths are canonical already. Scope checks must
        // precede I/O so a disconnected unrelated notebook cannot break this query.
        if within.is_some_and(|path| !path.starts_with(&notebook.path) && !notebook.path.starts_with(path)) {
            continue;
        }
        let root = std::fs::canonicalize(&notebook.path).map_err(|e| crate::StoreError::io("read tag scope", &notebook.path, e))?;
        query.visit(&root, &root, &notebook.info.title, None)?;
    }
    Ok(query.hits)
}

struct TagQuery<'a> {
    within: Option<&'a Path>,
    exact_page: bool,
    incomplete: bool,
    seen: std::collections::HashSet<PathBuf>,
    hits: Vec<TagHit>,
}

impl TagQuery<'_> {
    fn visit(&mut self, dir: &Path, notebook: &Path, notebook_title: &str, section: Option<(&Path, &str)>) -> crate::Result<()> {
        if self.within.is_some_and(|within| !dir.starts_with(within) && !within.starts_with(dir)) || !self.seen.insert(dir.to_path_buf()) {
            return Ok(());
        }
        if !dir.starts_with(notebook) {
            return Err(crate::StoreError::invalid(format!("tag scope reaches outside its notebook: {}", dir.display())));
        }
        let info = read_node(dir);
        let is_page = page_file(dir).is_some();
        if info.is_none() && !is_page {
            if std::fs::metadata(dir.join("_node.toml")).is_ok() {
                return Err(crate::StoreError::invalid(format!("cannot read tag scope metadata: {}", dir.display())));
            }
            return Ok(());
        }
        let section = match info.as_ref().filter(|n| n.kind == notecraft_model::NodeKind::Section) {
            Some(node) => Some((dir, node.title.as_str())),
            None => section,
        };
        if is_page && self.within.is_none_or(|within| dir.starts_with(within)) {
            let page = load_page(dir)?;
            let blocks = boxes::read(page.format, &page.body).0;
            for (block, content) in blocks.iter().enumerate() {
                for (tag, done, text) in tags_of_block(content) {
                    if self.incomplete && (tag != notecraft_model::Tag::ToDo || done != Some(false)) {
                        continue;
                    }
                    self.hits.push(TagHit {
                        path: dir.to_path_buf(),
                        title: page.title.clone(),
                        section_path: section.map(|(path, _)| path.to_path_buf()),
                        section: section.map(|(_, title)| title.to_owned()).unwrap_or_default(),
                        notebook_path: notebook.to_path_buf(),
                        notebook: notebook_title.to_owned(),
                        block,
                        tag,
                        text,
                        done,
                    });
                }
            }
        }
        if self.exact_page && self.within == Some(dir) {
            return Ok(());
        }
        let entries = std::fs::read_dir(dir).map_err(|e| crate::StoreError::io("read tag scope", dir, e))?;
        let mut children = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| crate::StoreError::io("read tag scope", dir, e))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "assets" || name.starts_with('.') {
                continue;
            }
            if self.within.is_some_and(|within| !path.starts_with(within) && !within.starts_with(&path)) {
                continue;
            }
            let meta = std::fs::metadata(&path).map_err(|e| crate::StoreError::io("read tag scope", &path, e))?;
            if meta.is_dir() {
                children.push(std::fs::canonicalize(&path).map_err(|e| crate::StoreError::io("read tag scope", &path, e))?);
            }
        }
        children.sort();
        for child in children {
            self.visit(&child, notebook, notebook_title, section)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub title: String,
    pub snippet: String,
}

fn collect_pages(dir: &Path, out: &mut Vec<PathBuf>) {
    if page_file(dir).is_some() {
        out.push(dir.to_path_buf());
    }
    for c in child_dirs(dir) {
        collect_pages(&c, out);
    }
}

/// Pages whose title or text contains `needle` (ignoring case). At most `limit` hits.
pub fn search(root: &Path, needle: &str, limit: usize) -> Vec<Hit> {
    search_in(root, &[], needle, limit)
}

/// [`search`] over the notebooks folder and the notebooks opened from elsewhere, each of which
/// is a notebook folder in its own right. Searched as one: a page is found once however it is
/// reached, and `limit` is for all of them together (issue #127).
pub fn search_in(root: &Path, elsewhere: &[PathBuf], needle: &str, limit: usize) -> Vec<Hit> {
    let needle = needle.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut pages = Vec::new();
    for nb in child_dirs(root).into_iter().chain(elsewhere.iter().cloned()) {
        collect_pages(&nb, &mut pages);
    }
    let mut seen = std::collections::HashSet::new();
    pages.retain(|page| seen.insert(std::fs::canonicalize(page).unwrap_or_else(|_| page.clone())));
    let mut hits = Vec::new();
    for dir in pages {
        let Ok(page) = load_page(&dir) else { continue };
        // The words a reader sees. Markdown is read into blocks and written as the HTML the
        // same page would be, so both formats are searched the same way: searched as source,
        // a phrase with a bold word in it, `Hello **world**`, was never found (issue #104).
        let plain = match page.format {
            PageFormat::Html => html::strip_tags(&page.body),
            PageFormat::Cmgfm => html::strip_tags(&render::blocks_to_html(&boxes::read(PageFormat::Cmgfm, &page.body).0)),
        };
        let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = read_node(&dir).map(|i| i.title).unwrap_or_default();
        let hay: Vec<char> = format!("{title} {plain}").to_lowercase().chars().collect();
        let n: Vec<char> = needle.chars().collect();
        if let Some(pos) = (0..hay.len().saturating_sub(n.len().saturating_sub(1))).find(|&i| hay.get(i..i + n.len()) == Some(n.as_slice())) {
            let from = pos.saturating_sub(30);
            let snippet: String = hay.iter().skip(from).take(100).collect::<String>().replace('\n', " ");
            hits.push(Hit { path: dir, title, snippet: snippet.split_whitespace().collect::<Vec<_>>().join(" ") });
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;
    use crate::page::save_body;
    use crate::settings::Settings;
    use crate::testing::TempDir;
    use notecraft_model::NodeKind;

    fn fixture(t: &TempDir) -> PathBuf {
        let s = Settings::at(t.path().join("s.toml"));
        let root = t.path().join("nb");
        let nb = ops::create_notebook(&root, "NB", None, &s).expect("nb");
        let a = ops::create_node(&nb, "Kickoff", NodeKind::Page, Some(PageFormat::Html), &s).expect("a");
        save_body(&a, "<h1>Kickoff</h1><p>We decide the <b>format</b> today.</p>").expect("save");
        let b = ops::create_node(&a, "Minutes", NodeKind::Page, Some(PageFormat::Cmgfm), &s).expect("b");
        save_body(&b, "# Minutes\n\n- [ ] Review the caf\u{e9} budget\n").expect("save");
        root
    }

    #[test]
    fn finds_text_in_both_formats_and_in_titles() {
        let t = TempDir::new("search1");
        let root = fixture(&t);
        assert_eq!(search(&root, "decide the format", 10).len(), 1, "tags between words are ignored");
        assert_eq!(search(&root, "CAF\u{c9}", 10).len(), 1, "case-insensitive, non-ASCII");
        assert_eq!(search(&root, "minutes", 10).len(), 1, "title match");
        assert!(search(&root, "absent", 10).is_empty());
    }

    /// Issue #104: on a CMGFM page a phrase is found whatever marks are round its words, as
    /// it is on an HTML page.
    #[test]
    fn a_phrase_across_formatting_is_found_on_a_markdown_page() {
        let t = TempDir::new("search-md-formatting");
        let s = Settings::at(t.path().join("s.toml"));
        let root = t.path().join("nb");
        let nb = ops::create_notebook(&root, "NB", None, &s).expect("nb");
        let md = ops::create_node(&nb, "Markdown", NodeKind::Page, Some(PageFormat::Cmgfm), &s).expect("md");
        save_body(&md, "Hello **world**, see the *new* [plan](https://example.test) and `the code`.\n").expect("save");
        let html = ops::create_node(&nb, "Html", NodeKind::Page, Some(PageFormat::Html), &s).expect("html");
        save_body(&html, "<p>Hello <b>world</b>, see the <i>new</i> <a href=\"https://example.test\">plan</a> and <code>the code</code>.</p>")
            .expect("save");
        for phrase in ["Hello world", "see the new plan and the code"] {
            let found: Vec<String> = search(&root, phrase, 10).into_iter().map(|hit| hit.title).collect();
            assert!(found.contains(&"Markdown".to_string()) && found.contains(&"Html".to_string()), "{phrase:?} on both pages: {found:?}");
        }
        assert!(search(&root, "**world**", 10).is_empty(), "the marks themselves are not words on the page");
    }

    #[test]
    fn limits_and_empty_needles() {
        let t = TempDir::new("search2");
        let root = fixture(&t);
        assert!(search(&root, "", 10).is_empty());
        assert!(search(&root, "   ", 10).is_empty());
        assert!(search(&root, "the", 0).is_empty());
        assert_eq!(search(&root, "the", 1).len(), 1);
    }

    #[test]
    fn snippets_are_short_single_lines() {
        let t = TempDir::new("search3");
        let root = fixture(&t);
        let hit = &search(&root, "review", 5)[0];
        assert!(hit.snippet.chars().count() <= 100 && !hit.snippet.contains('\n'), "{:?}", hit.snippet);
    }

    #[test]
    fn a_missing_root_is_empty_not_an_error() {
        assert!(search(Path::new("/definitely/not/here"), "x", 5).is_empty());
    }

    /// Issue #105: a word that is partly bold on an HTML page is found as the word it reads as.
    #[test]
    fn a_partly_formatted_word_on_an_html_page_is_found() {
        let t = TempDir::new("search-html-inline");
        let s = Settings::at(t.path().join("s.toml"));
        let root = t.path().join("nb");
        let nb = ops::create_notebook(&root, "NB", None, &s).expect("nb");
        let page = ops::create_node(&nb, "Plain", NodeKind::Page, Some(PageFormat::Html), &s).expect("page");
        save_body(&page, "<p>The for<b>mat</b> of co<i>op</i>eration</p>").expect("save");
        assert_eq!(search(&root, "format", 10).len(), 1);
        assert_eq!(search(&root, "the format of cooperation", 10).len(), 1);
    }
}
