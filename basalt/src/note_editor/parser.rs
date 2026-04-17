use std::ops::{Deref, DerefMut};

use pulldown_cmark::{CodeBlockKind, Event, Options, Tag, TagEnd};

use crate::note_editor::{
    ast::{self, Node, SourceRange, TaskKind},
    rich_text::{RichText, Style, TextSegment},
};

pub struct Parser<'a>(pulldown_cmark::TextMergeWithOffset<'a, pulldown_cmark::OffsetIter<'a>>);

impl<'a> Deref for Parser<'a> {
    type Target = pulldown_cmark::TextMergeWithOffset<'a, pulldown_cmark::OffsetIter<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Parser<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = (Event<'a>, SourceRange<usize>);
    fn next(&mut self) -> Option<Self::Item> {
        self.deref_mut().next()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ParserState {
    task_kind: Vec<ast::TaskKind>,
    item_kind: Vec<ast::ItemKind>,
}

impl<'a> Parser<'a> {
    /// Creates a new [`Parser`] from a Markdown input string.
    ///
    /// The parser uses [`pulldown_cmark::Parser::new_ext`] with [`Options::all()`] and
    /// [`pulldown_cmark::TextMergeWithOffset`] internally.
    ///
    /// The offset is required to know where the node appears in the provided source text.
    pub fn new(text: &'a str) -> Self {
        let mut options = Options::all();

        // Smart punctuation is excluded because it converts ASCII characters (e.g. ", ') to
        // multi-byte Unicode (“, ‘), making the rendered text longer than the source, causing the
        // source offset to overlap in some cases and causing unexpected behavior.
        //
        // TODO: Holistic approach to support smart punctation. Potentially need to do this on a
        // different layer of the app to only do the smart punctuation effect visually using
        // virtual elements or such, but keeping the original source content unchanged.
        options.remove(Options::ENABLE_SMART_PUNCTUATION);

        let parser = pulldown_cmark::TextMergeWithOffset::new(
            pulldown_cmark::Parser::new_ext(text, options).into_offset_iter(),
        );

        Self(parser)
    }

    pub fn parse(mut self) -> Vec<Node> {
        let mut result = Vec::new();
        let mut state = ParserState::default();

        while let Some((event, _)) = self.next() {
            match event {
                Event::Start(tag) if Self::is_container_tag(&tag) => {
                    if let Some(node) = self.parse_container(tag, &mut state) {
                        result.push(node);
                    }
                }
                _ => {}
            }
        }

        result
    }

    pub fn parse_container(&mut self, tag: Tag, state: &mut ParserState) -> Option<Node> {
        let mut nodes = Vec::new();
        let mut text_segments = Vec::new();
        let mut inline_styles = Vec::new();

        match tag {
            Tag::List(Some(start)) => {
                state.item_kind.push(ast::ItemKind::Ordered(start));
            }
            Tag::List(..) => {
                state.item_kind.push(ast::ItemKind::Unordered);
            }
            _ => {}
        };

        while let Some((event, source_range)) = self.next() {
            match event {
                Event::Start(inner_tag) if Self::is_container_tag(&inner_tag) => {
                    if let Some(node) = self.parse_container(inner_tag, state) {
                        nodes.push(node);
                    }
                }

                Event::Start(inner_tag) if Self::is_inline_tag(&inner_tag) => {
                    if let Some(style) = Self::tag_to_style(&inner_tag) {
                        inline_styles.push(style);
                    }
                }

                Event::TaskListMarker(checked) => {
                    state.task_kind.push(if checked {
                        TaskKind::Checked
                    } else {
                        TaskKind::Unchecked
                    });
                }

                Event::Code(text) => {
                    let text_segment = TextSegment::styled(&text, Style::Code);
                    text_segments.push(text_segment);
                }

                Event::Text(text) => {
                    let mut text_segment = TextSegment::plain(&text);
                    inline_styles.iter().for_each(|style| {
                        text_segment.add_style(style);
                    });
                    text_segments.push(text_segment);
                }

                Event::SoftBreak => {
                    let text_segment = TextSegment::empty_line();
                    text_segments.push(text_segment);
                }

                Event::End(tag_end) if Self::tags_match(&tag, &tag_end) => {
                    let text = if !text_segments.is_empty() {
                        RichText::from(text_segments)
                    } else {
                        RichText::empty()
                    };

                    return match tag {
                        Tag::Heading { level, .. } => Some(Node::Heading {
                            level: level.into(),
                            text,
                            source_range,
                        }),
                        Tag::Item => {
                            // This is required since in block quotes list items are considered
                            // "tight", thus the text is not stored in a paragraph directly.
                            // TODO: Think if wrapping this into a paragraph is a good idea or not.
                            // Potentially storing a RichText here is better.
                            if !text.is_empty() {
                                nodes.insert(
                                    0,
                                    Node::Paragraph {
                                        text,
                                        source_range: source_range.clone(),
                                    },
                                );
                            }

                            let item = if let Some(kind) = state.task_kind.pop() {
                                Some(Node::Task {
                                    kind,
                                    nodes,
                                    source_range,
                                })
                            } else {
                                Some(Node::Item {
                                    kind: state
                                        .item_kind
                                        .last()
                                        .cloned()
                                        .unwrap_or(ast::ItemKind::Unordered),
                                    nodes,
                                    source_range,
                                })
                            };

                            if let Some(ast::ItemKind::Ordered(start)) = state.item_kind.last_mut()
                            {
                                *start += 1;
                            };

                            item
                        }
                        Tag::List(..) => {
                            state.item_kind.pop();

                            Some(Node::List {
                                nodes,
                                source_range,
                            })
                        }
                        Tag::CodeBlock(kind) => Some(Node::CodeBlock {
                            lang: match kind {
                                CodeBlockKind::Fenced(lang) => Some(lang.to_string()),
                                _ => None,
                            },
                            text,
                            source_range,
                        }),
                        Tag::BlockQuote(kind) => {
                            let mut resolved_kind = kind.map(ast::BlockQuoteKind::from);
                            let mut title: Option<String> = None;

                            // ITS Theme detection: only when pulldown-cmark did not recognize the
                            // type (kind == None). Standard types (Note/Tip/etc.) are already
                            // consumed by pulldown-cmark and their [!TYPE] line is removed.
                            //
                            // When `> [!aside]\n> body text` is parsed, pulldown-cmark merges
                            // both lines into a single tight Paragraph with a SoftBreak event
                            // (which becomes '\n' in the RichText). We detect the [!type] pattern
                            // only on the first line, and if body content follows after '\n',
                            // we re-insert it as a new Paragraph node in the nodes list.
                            if resolved_kind.is_none() {
                                if let Some(first_text) =
                                    nodes.first().and_then(extract_paragraph_text)
                                {
                                    // Only examine the first line (before any SoftBreak newline).
                                    let (first_line, remainder) =
                                        match first_text.split_once('\n') {
                                            Some((first, rest)) => (first.trim(), rest.trim()),
                                            None => (first_text.trim(), ""),
                                        };
                                    if let Some(rest) = first_line.strip_prefix("[!") {
                                        if let Some(bracket_end) = rest.find(']') {
                                            let type_str = &rest[..bracket_end];
                                            let after_bracket =
                                                rest[bracket_end + 1..].trim().to_string();
                                            if let Some(detected_kind) = its_theme_kind(type_str) {
                                                resolved_kind = Some(detected_kind);
                                                title = if after_bracket.is_empty() {
                                                    None
                                                } else {
                                                    Some(after_bracket)
                                                };
                                                let first_range =
                                                    nodes[0].source_range().clone();
                                                nodes.remove(0);
                                                // Re-insert body content after the [!type] line
                                                // (merged by SoftBreak) as a new Paragraph node.
                                                if !remainder.is_empty() {
                                                    nodes.insert(
                                                        0,
                                                        Node::Paragraph {
                                                            text: RichText::from(
                                                                [TextSegment::plain(remainder)],
                                                            ),
                                                            source_range: first_range,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Some(Node::BlockQuote {
                                kind: resolved_kind,
                                title,
                                nodes,
                                source_range,
                            })
                        }
                        Tag::Paragraph => Some(Node::Paragraph { text, source_range }),
                        _ => None,
                    };
                }
                _ => {}
            }
        }

        None
    }

    fn is_container_tag(tag: &Tag) -> bool {
        matches!(
            tag,
            Tag::Paragraph
                | Tag::Item
                | Tag::List(..)
                | Tag::BlockQuote(..)
                | Tag::CodeBlock(..)
                | Tag::Heading { .. }
        )
    }

    fn is_inline_tag(tag: &Tag) -> bool {
        matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
    }

    fn tags_match(start: &Tag, end: &TagEnd) -> bool {
        fn tag_to_end(tag: &Tag) -> Option<TagEnd> {
            match tag {
                Tag::Heading { level, .. } => Some(TagEnd::Heading(*level)),
                Tag::List(ordered) => Some(TagEnd::List(ordered.is_some())),
                Tag::Item => Some(TagEnd::Item),
                Tag::BlockQuote(kind) => Some(TagEnd::BlockQuote(*kind)),
                Tag::CodeBlock(..) => Some(TagEnd::CodeBlock),
                Tag::Paragraph => Some(TagEnd::Paragraph),
                _ => None,
            }
        }

        if let Some(start) = tag_to_end(start) {
            std::mem::discriminant(&start) == std::mem::discriminant(end)
        } else {
            false
        }
    }

    fn tag_to_style(tag: &Tag) -> Option<Style> {
        match tag {
            Tag::Emphasis => Some(Style::Emphasis),
            Tag::Strong => Some(Style::Strong),
            Tag::Strikethrough => Some(Style::Strikethrough),
            _ => None,
        }
    }
}

/// Maps an ITS Theme callout type string (case-insensitive) to a [`ast::BlockQuoteKind`] variant.
/// Returns [`None`] for unrecognized type strings (treated as plain blockquotes).
/// Handles aliases: caption/captions, column/columns, quote/quotes.
fn its_theme_kind(type_str: &str) -> Option<ast::BlockQuoteKind> {
    match type_str.to_ascii_lowercase().as_str() {
        // Standard GitHub Alert types (for case-insensitive support)
        "note" => Some(ast::BlockQuoteKind::Note),
        "tip" => Some(ast::BlockQuoteKind::Tip),
        "important" => Some(ast::BlockQuoteKind::Important),
        "warning" => Some(ast::BlockQuoteKind::Warning),
        "caution" => Some(ast::BlockQuoteKind::Caution),
        // ITS Theme Extended (post-processing detection)
        "aside" => Some(ast::BlockQuoteKind::Aside),
        "blank" => Some(ast::BlockQuoteKind::Blank),
        "caption" | "captions" => Some(ast::BlockQuoteKind::Caption),
        "cards" => Some(ast::BlockQuoteKind::Cards),
        "checks" => Some(ast::BlockQuoteKind::Checks),
        "column" | "columns" => Some(ast::BlockQuoteKind::Column),
        "grid" => Some(ast::BlockQuoteKind::Grid),
        "infobox" => Some(ast::BlockQuoteKind::Infobox),
        "kanban" => Some(ast::BlockQuoteKind::Kanban),
        "kith" => Some(ast::BlockQuoteKind::Kith),
        "metadata" => Some(ast::BlockQuoteKind::Metadata),
        "quote" | "quotes" => Some(ast::BlockQuoteKind::Quote),
        "recite" => Some(ast::BlockQuoteKind::Recite),
        "statblocks" => Some(ast::BlockQuoteKind::Statblocks),
        "timeline" => Some(ast::BlockQuoteKind::Timeline),
        _ => None,
    }
}

/// Extracts the text content of a [`Node::Paragraph`] as a [`String`].
/// Returns [`None`] if the node is not a paragraph.
fn extract_paragraph_text(node: &ast::Node) -> Option<String> {
    if let ast::Node::Paragraph { text, .. } = node {
        Some(text.to_string())
    } else {
        None
    }
}

pub fn from_str(text: &str) -> Vec<Node> {
    Parser::new(text).parse()
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_parser() {
        let tests = [
            (
                "paragraphs",
                indoc! { r#"## Paragraphs
                To create paragraphs in Markdown, use a **blank line** to separate blocks of text. Each block of text separated by a blank line is treated as a distinct paragraph.

                This is a paragraph.

                This is another paragraph.

                A blank line between lines of text creates separate paragraphs. This is the default behavior in Markdown.
                "#},
            ),
            (
                "headings",
                indoc! { r#"## Headings
                To create a heading, add up to six `#` symbols before your heading text. The number of `#` symbols determines the size of the heading.

                # This is a heading 1
                ## This is a heading 2
                ### This is a heading 3
                #### This is a heading 4
                ##### This is a heading 5
                ###### This is a heading 6
                "#},
            ),
            (
                "lists",
                indoc! { r#"## Lists
                You can create an unordered list by adding a `-`, `*`, or `+` before the text.

                - First list item
                - Second list item
                - Third list item

                To create an ordered list, start each line with a number followed by a `.` or `)` symbol.

                1. First list item
                2. Second list item
                3. Third list item

                1) First list item
                2) Second list item
                3) Third list item
                "#},
            ),
            (
                "lists_line_breaks",
                indoc! { r#"## Lists with line breaks
                You can use line breaks within an ordered list without altering the numbering.

                1. First list item

                2. Second list item
                3. Third list item

                4. Fourth list item
                5. Fifth list item
                6. Sixth list item
                "#},
            ),
            (
                "task_lists",
                indoc! { r#"## Task lists
                To create a task list, start each list item with a hyphen and space followed by `[ ]`.

                - [x] This is a completed task.
                - [ ] This is an incomplete task.

                You can toggle a task in Reading view by selecting the checkbox.

                > [!tip]
                > You can use any character inside the brackets to mark it as complete.
                >
                > - [x] Milk
                > - [?] Eggs
                > - [-] Eggs
                "#},
            ),
            (
                "nesting_lists",
                indoc! { r#"## Nesting lists
                You can nest any type of list—ordered, unordered, or task lists—under any other type of list.

                To create a nested list, indent one or more list items. You can mix list types within a nested structure:

                1. First list item
                   1. Ordered nested list item
                2. Second list item
                   - Unordered nested list item
                "#},
            ),
            (
                "nesting_task_lists",
                indoc! { r#"## Nesting task lists
                Similarly, you can create a nested task list by indenting one or more list items:

                - [ ] Task item 1
                  - [ ] Subtask 1
                - [ ] Task item 2
                  - [ ] Subtask 2
                "#},
            ),
            // TODO: Implement horizontal rule
            // (
            //     "horizontal_rule",
            //     indoc! { r#"## Horizontal rule
            //     You can use three or more stars `***`, hyphens `---`, or underscore `___` on its own line to add a horizontal bar. You can also separate symbols using spaces.
            //
            //     ***
            //     ****
            //     * * *
            //     ---
            //     ----
            //     - - -
            //     ___
            //     ____
            //     _ _ _
            //     "#},
            // ),
            (
                "code_blocks",
                indoc! { r#"## Code blocks
                To format code as a block, enclose it with three backticks or three tildes.

                ```md
                cd ~/Desktop
                ```

                You can also create a code block by indenting the text using `Tab` or 4 blank spaces.

                    cd ~/Desktop

                "#},
            ),
            (
                "code_syntax_highlighting_in_blocks",
                indoc! { r#"## Code syntax highlighting in blocks
                You can add syntax highlighting to a code block, by adding a language code after the first set of backticks.

                ```js
                function fancyAlert(arg) {
                  if(arg) {
                    $.facebox({div:'#foo'})
                  }
                }
                ```
                "#},
            ),
        ];

        tests.into_iter().for_each(|(name, text)| {
            assert_snapshot!(
                name,
                format!(
                    "{}\n ---\n\n{}",
                    text,
                    ast::nodes_to_sexp(&from_str(text), 0)
                )
            );
        });
    }

    /// Parse a blockquote input and return (kind, title, body_texts).
    fn parse_bq(input: &str) -> (Option<ast::BlockQuoteKind>, Option<String>, Vec<String>) {
        let nodes = from_str(input);
        assert_eq!(nodes.len(), 1, "expected one top-level node");
        match nodes.into_iter().next().unwrap() {
            Node::BlockQuote { kind, title, nodes, .. } => {
                let texts = nodes
                    .into_iter()
                    .filter_map(|n| match n {
                        Node::Paragraph { text, .. } => Some(text.to_string()),
                        _ => None,
                    })
                    .collect();
                (kind, title, texts)
            }
            _ => panic!("expected BlockQuote"),
        }
    }

    #[test]
    fn its_theme_aside_callout() {
        let (kind, title, body) = parse_bq("> [!aside]\n> body text");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Aside));
        assert_eq!(title, None);
        assert_eq!(body, vec!["body text"]);
    }

    #[test]
    fn its_theme_kanban_with_title() {
        let (kind, title, body) = parse_bq("> [!kanban] My Board\n> body text");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Kanban));
        assert_eq!(title, Some("My Board".to_string()));
        assert_eq!(body, vec!["body text"]);
    }

    #[test]
    fn its_theme_case_insensitive() {
        let (kind, title, body) = parse_bq("> [!ASIDE]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Aside));
        assert_eq!(title, None);
        assert_eq!(body, vec!["body"]);
    }

    #[test]
    fn its_theme_aliases() {
        let (kind, _, _) = parse_bq("> [!captions]\n> text");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Caption));

        let (kind2, _, _) = parse_bq("> [!columns]\n> text");
        assert_eq!(kind2, Some(ast::BlockQuoteKind::Column));

        let (kind3, _, _) = parse_bq("> [!quotes]\n> text");
        assert_eq!(kind3, Some(ast::BlockQuoteKind::Quote));
    }

    #[test]
    fn plain_blockquote_unchanged() {
        let (kind, title, _) = parse_bq("> plain text");
        assert_eq!(kind, None);
        assert_eq!(title, None);
    }

    #[test]
    fn unknown_type_stays_plain() {
        let (kind, title, _) = parse_bq("> [!unknowntype]\n> body");
        assert_eq!(kind, None);
        assert_eq!(title, None);
    }

    #[test]
    fn standard_callout_kind_unchanged() {
        // pulldown-cmark natively recognizes [!tip], kind is Some(Tip)
        let (kind, title, body) = parse_bq("> [!tip]\n> body text");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Tip));
        assert_eq!(title, None);
        assert_eq!(body, vec!["body text"]);
    }

    #[test]
    fn standard_callout_case_insensitive() {
        // Verify ITS Theme detection handles lowercase standard types.
        let (kind, title, body) = parse_bq("> [!note]\n> body text");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Note));
        assert_eq!(title, None);
        assert_eq!(body, vec!["body text"]);
    }

    #[test]
    fn standard_callout_all_types_support() {
        // Test all 5 standard types with lowercase.
        let (kind, _, _) = parse_bq("> [!note]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Note));

        let (kind, _, _) = parse_bq("> [!tip]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Tip));

        let (kind, _, _) = parse_bq("> [!important]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Important));

        let (kind, _, _) = parse_bq("> [!warning]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Warning));

        let (kind, _, _) = parse_bq("> [!caution]\n> body");
        assert_eq!(kind, Some(ast::BlockQuoteKind::Caution));
    }
}
