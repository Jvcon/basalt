use ratatui::{
    style::{Color, Modifier, Style, Stylize},
    text::Span,
};

use unicode_width::UnicodeWidthStr;

use crate::{
    app::SyntectContext,
    config::Symbols,
    note_editor::{
        ast::{self, SourceRange},
        rich_text::RichText,
        text_wrap::wrap_preserve_trailing,
        virtual_document::{
            content_span, empty_virtual_line, synthetic_span, virtual_line, VirtualBlock,
            VirtualLine, VirtualSpan,
        },
    },
    stylized_text::stylize,
};

trait SpanExt {
    fn merge(self, other: Span) -> Span;
}

impl SpanExt for &Span<'_> {
    fn merge(self, other: Span) -> Span {
        Span::styled(
            format!("{}{}", self.content, other.content),
            self.style.patch(other.style),
        )
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum RenderStyle {
    Raw,
    Visual,
}

// Internal consolidated text wrapping function
// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
fn text_wrap_internal<'a>(
    text_content: &str,
    text_style: Style,
    prefix: Span<'static>,
    source_range: &SourceRange<usize>,
    width: usize,
    marker: Option<Span<'static>>,
    option: &RenderStyle,
    symbols: &Symbols,
) -> Vec<VirtualLine<'a>> {
    let wrap_marker = &symbols.wrap_marker;
    let wrapped_lines = wrap_preserve_trailing(text_content, width, wrap_marker.width() + 1);

    let mut current_range_start = source_range.start;

    wrapped_lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line_byte_len = line.len();

            let line_source_range =
                current_range_start..(current_range_start + line_byte_len).min(source_range.end);

            current_range_start += line_byte_len;

            let first_line = i == 0;
            let content_span = Span::styled(line.to_string(), text_style);

            match (&marker, first_line) {
                (Some(marker), true) if *option == RenderStyle::Visual => virtual_line!([
                    synthetic_span!(prefix),
                    synthetic_span!(marker),
                    content_span!(content_span, line_source_range)
                ]),
                (_, true) => virtual_line!([
                    synthetic_span!(prefix),
                    content_span!(content_span, line_source_range)
                ]),
                _ => {
                    let marker_padding = marker.as_ref().map(|m| m.width()).unwrap_or(0);
                    virtual_line!([
                        synthetic_span!(prefix),
                        synthetic_span!(Span::styled(" ".repeat(marker_padding), prefix.style)),
                        synthetic_span!(Span::styled(wrap_marker.clone(), Style::new().black())),
                        content_span!(content_span, line_source_range)
                    ])
                }
            }
        })
        .collect()
}

fn render_raw_line<'a>(
    line: &str,
    prefix: Span<'static>,
    source_range: &SourceRange<usize>,
    max_width: usize,
    symbols: &Symbols,
) -> Vec<VirtualLine<'a>> {
    text_wrap_internal(
        // TODO: Replace with `»` as a synthetic symbol for tabs
        // Tab characters need to be replaced to spaces or other characters as the tab characters
        // will break the UI. Similarly the same issue that I was facing was solved by replacing
        // the tab characters: https://github.com/ratatui/ratatui/issues/1606#issuecomment-3172769529
        &line.replace("\t", "  "),
        Style::default(),
        prefix,
        source_range,
        max_width,
        None,
        &RenderStyle::Raw,
        symbols,
    )
}

// # Example:
//
// | Basalt is a TUI (Terminal User Interface)
// |  ⤷ application to manage Obsidian vaults and
// |  ⤷ notes from the terminal. Basalt is
// |  ⤷ cross-platform and can be installed and run
// |  ⤷ in the major operating systems on Windows,
// |  ⤷ macOS; and Linux.
pub fn text_wrap<'a>(
    text: &Span<'a>,
    prefix: Span<'static>,
    source_range: &SourceRange<usize>,
    width: usize,
    marker: Option<Span<'static>>,
    option: &RenderStyle,
    symbols: &Symbols,
) -> Vec<VirtualLine<'a>> {
    text_wrap_internal(
        &text.content,
        text.style,
        prefix,
        source_range,
        width,
        marker,
        option,
        symbols,
    )
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn heading<'a>(
    level: ast::HeadingLevel,
    content: &str,
    prefix: Span<'static>,
    text: &RichText,
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
) -> VirtualBlock<'a> {
    use ast::HeadingLevel::*;
    // FIXME: Support new lines when editing
    // Currently when editing the heading and inserting new lines, the new lines are invisible and
    // only take affect visually when exiting (commiting edit changes)
    let text = text.to_string();
    let text = match option {
        RenderStyle::Visual => text,
        RenderStyle::Raw => content.to_string(),
    };

    let prefix_width = prefix.width();

    let h = |marker: Span<'static>, content: Span<'a>| {
        let mut wrapped_heading = text_wrap(
            &content,
            prefix.clone(),
            source_range,
            max_width,
            Some(marker),
            option,
            symbols,
        );

        wrapped_heading.push(empty_virtual_line!());
        wrapped_heading
    };

    let h_with_underline = |content: Span<'a>, underline: Span<'static>| {
        let mut wrapped_heading = text_wrap(
            &content,
            prefix.clone(),
            source_range,
            max_width,
            None,
            option,
            symbols,
        );
        wrapped_heading.push(virtual_line!([synthetic_span!(underline)]));
        wrapped_heading
    };

    let lines = match level {
        H1 => h_with_underline(
            if *option == RenderStyle::Visual {
                text.to_uppercase().bold()
            } else {
                text.bold()
            },
            symbols
                .h1_underline
                .repeat(max_width.saturating_sub(prefix_width))
                .into(),
        ),
        H2 => h_with_underline(
            text.bold().yellow(),
            symbols
                .h2_underline
                .repeat(max_width.saturating_sub(prefix_width))
                .yellow(),
        ),
        H3 => h(format!("{} ", symbols.h3_marker).cyan(), text.bold().cyan()),
        H4 => h(
            format!("{} ", symbols.h4_marker).magenta(),
            text.bold().magenta(),
        ),
        H5 => h(
            format!("{} ", symbols.h5_marker).into(),
            match symbols.h5_font_style {
                Some(style) => stylize(&text, style).into(),
                None => text.into(),
            },
        ),
        H6 => h(
            format!("{} ", symbols.h6_marker).into(),
            match symbols.h6_font_style {
                Some(style) => stylize(&text, style).into(),
                None => text.into(),
            },
        ),
    };

    VirtualBlock::new(&lines, source_range)
}

pub fn render_raw<'a>(
    content: &str,
    source_range: &SourceRange<usize>,
    max_width: usize,
    prefix: Span<'static>,
    symbols: &Symbols,
) -> Vec<VirtualLine<'a>> {
    let mut current_range_start = source_range.start;

    let mut lines = content
        .lines()
        .flat_map(|line| {
            // TODO: Make sure that the line cannot exceed the source range end
            let line_range = line_range(current_range_start, line.len(), true);
            current_range_start = line_range.end;

            if line.is_empty() {
                vec![virtual_line!([
                    synthetic_span!(prefix.clone()),
                    content_span!("".to_string(), line_range)
                ])]
            } else {
                render_raw_line(line, prefix.clone(), &line_range, max_width, symbols)
            }
        })
        .collect::<Vec<_>>();

    // When content is empty (e.g. empty file), produce a content line so the
    // cursor has something to land on.
    if lines.is_empty() {
        lines.push(virtual_line!([
            synthetic_span!(prefix),
            content_span!("".to_string(), source_range)
        ]));
    }

    lines.push(empty_virtual_line!());
    lines
}

pub fn paragraph<'a>(
    content: &str,
    prefix: Span<'static>,
    text: &RichText,
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => render_raw(content, source_range, max_width, prefix, symbols),
        RenderStyle::Visual => {
            let text = text.to_string();
            let mut current_range_start = source_range.start;

            let mut lines = text
                .to_string()
                .lines()
                .flat_map(|line| {
                    let line_range = line_range(current_range_start, line.len(), true);
                    current_range_start = line_range.end;

                    text_wrap(
                        &line.to_string().into(),
                        prefix.clone(),
                        &line_range,
                        max_width,
                        None,
                        option,
                        symbols,
                    )
                })
                .collect::<Vec<_>>();

            if prefix.to_string().is_empty() {
                lines.extend([empty_virtual_line!()]);
            }

            lines
        }
    };

    VirtualBlock::new(&lines, source_range)
}

#[allow(clippy::too_many_arguments)]
pub fn code_block<'a>(
    content: &str,
    prefix: Span<'static>,
    // TODO: Add lang support
    // Ref: https://github.com/erikjuhani/basalt/issues/96
    _lang: &Option<String>,
    _syntect_ctx: Option<&SyntectContext>,
    text: &RichText,
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => {
            let mut current_range_start = source_range.start;

            let mut lines = content
                .lines()
                .map(|line| {
                    let line_range = line_range(current_range_start, line.len(), true);
                    current_range_start = line_range.end;

                    virtual_line!([
                        synthetic_span!(prefix.clone()),
                        synthetic_span!(Span::styled(" ", Style::new().bg(Color::Black))),
                        content_span!(line.to_string().bg(Color::Black), line_range),
                        synthetic_span!(" "
                            .repeat(
                                max_width
                                    .saturating_sub(prefix.width() + line.chars().count())
                                    .saturating_sub(1)
                            )
                            .bg(Color::Black)),
                    ])
                })
                .collect::<Vec<_>>();

            lines.push(empty_virtual_line!());
            lines
        }
        RenderStyle::Visual => {
            let text = text.to_string();

            let padding_line = virtual_line!([
                synthetic_span!(prefix.clone()),
                synthetic_span!(" "
                    .repeat(max_width.saturating_sub(prefix.width()))
                    .bg(Color::Black))
            ]);

            let mut current_range_start = source_range.start;

            let mut lines = vec![padding_line.clone()];
            lines.extend(text.lines().map(|line| {
                let source_range = line_range(current_range_start, line.len(), true);
                current_range_start = source_range.end;

                virtual_line!([
                    synthetic_span!(prefix.clone()),
                    synthetic_span!(Span::styled(" ", Style::new().bg(Color::Black))),
                    content_span!(line.to_string().bg(Color::Black), source_range),
                    synthetic_span!(" "
                        .repeat(
                            max_width
                                .saturating_sub(prefix.width() + line.chars().count())
                                .saturating_sub(1)
                        )
                        .bg(Color::Black)),
                ])
            }));
            lines.extend([padding_line]);
            lines.extend([empty_virtual_line!()]);
            lines
        }
    };

    VirtualBlock::new(&lines, source_range)
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn list<'a>(
    content: &str,
    prefix: Span<'static>,
    nodes: &[ast::Node],
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
    list_depth: usize,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => render_raw(content, source_range, max_width, prefix, symbols),
        RenderStyle::Visual => {
            let mut lines: Vec<VirtualLine<'a>> = nodes
                .iter()
                .flat_map(|node| {
                    let node_content = content
                        .get(node.source_range().clone())
                        .unwrap_or("")
                        .to_string();
                    render_node(
                        node_content,
                        node,
                        max_width,
                        prefix.clone(),
                        option,
                        symbols,
                        list_depth,
                        None,
                    )
                    .lines
                })
                .collect();

            if prefix.to_string().is_empty() {
                lines.extend([empty_virtual_line!()]);
            }
            lines
        }
    };

    VirtualBlock::new(&lines, source_range)
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn task<'a>(
    content: &str,
    prefix: Span<'static>,
    kind: &ast::TaskKind,
    nodes: &[ast::Node],
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
    list_depth: usize,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => render_raw(content, source_range, max_width, prefix, symbols),
        RenderStyle::Visual => {
            let Some((text, rest)) = nodes.split_first().and_then(|(first, rest)| {
                let text = first.rich_text()?;
                Some((text, rest))
            }) else {
                return VirtualBlock::new(&[], source_range);
            };

            let text = text.to_string();
            let text = match option {
                RenderStyle::Visual => text,
                RenderStyle::Raw => content.to_string(),
            };
            let (marker, text) = match kind {
                ast::TaskKind::Unchecked => (
                    format!("{} ", symbols.task_unchecked).dark_gray(),
                    text.into(),
                ),
                ast::TaskKind::LooselyChecked => (
                    format!("{} ", symbols.task_checked).magenta(),
                    text.dark_gray(),
                ),
                ast::TaskKind::Checked => (
                    format!("{} ", symbols.task_checked).magenta(),
                    text.dark_gray().add_modifier(Modifier::CROSSED_OUT),
                ),
            };

            let mut lines = text_wrap(
                &text,
                prefix.clone(),
                source_range,
                max_width,
                Some(marker),
                option,
                symbols,
            );

            lines.extend(rest.iter().flat_map(|node| {
                render_node(
                    content.to_string(),
                    node,
                    max_width,
                    prefix.merge("  ".into()),
                    option,
                    symbols,
                    list_depth + 1,
                    None,
                )
                .lines
            }));

            lines
        }
    };

    VirtualBlock::new(&lines, source_range)
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn item<'a>(
    content: &str,
    prefix: Span<'static>,
    kind: &ast::ItemKind,
    nodes: &[ast::Node],
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
    list_depth: usize,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => render_raw(content, source_range, max_width, prefix, symbols),
        RenderStyle::Visual => {
            let Some((text, rest)) = nodes.split_first().and_then(|(first, rest)| {
                let text = first.rich_text()?;
                Some((text, rest))
            }) else {
                return VirtualBlock::new(&[], source_range);
            };

            let text = text.to_string();

            let marker = match kind {
                ast::ItemKind::Ordered(i) => format!("{i}. ").dark_gray(),
                ast::ItemKind::Unordered => {
                    let marker = if symbols.list_markers.is_empty() {
                        "-"
                    } else {
                        &symbols.list_markers[list_depth % symbols.list_markers.len()]
                    };
                    format!("{marker} ").dark_gray()
                }
            };

            let mut lines = text_wrap(
                &text.into(),
                // TODO: Make the visual marker a separate prefix so we do not repeat it
                prefix.clone(),
                source_range,
                max_width,
                Some(marker),
                option,
                symbols,
            );

            lines.extend(rest.iter().flat_map(|node| {
                render_node(
                    content.to_string(),
                    node,
                    max_width,
                    prefix.merge("  ".into()),
                    option,
                    symbols,
                    list_depth + 1,
                    None,
                )
                .lines
            }));

            lines
        }
    };

    VirtualBlock::new(&lines, source_range)
}

pub fn line_range(start: usize, line_width: usize, newline: bool) -> SourceRange<usize> {
    // NOTE: When the content is replaced by rope the new lines are kept
    // + 1 for newline
    let end = start + line_width + if newline { 1 } else { 0 };
    start..end
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn block_quote<'a>(
    content: &str,
    prefix: Span<'static>,
    kind: &Option<ast::BlockQuoteKind>,
    title: &Option<String>,
    nodes: &[ast::Node],
    source_range: &SourceRange<usize>,
    max_width: usize,
    option: &RenderStyle,
    symbols: &Symbols,
) -> VirtualBlock<'a> {
    let lines = match option {
        RenderStyle::Raw => render_raw(content, source_range, max_width, prefix, symbols),
        RenderStyle::Visual => {
            let mut lines = Vec::new();
            let border_color = kind.as_ref().map(|k| callout_style(k).0).unwrap_or(Color::Magenta);

            // Header line for callouts (D-05, D-07)
            if let Some(ref k) = kind {
                let (color, _, _, _) = callout_style(k);
                let icon = callout_icon(k, &symbols.preset);
                let type_name = callout_type_name(k);
                let header_text = match title {
                    Some(t) => format!("{} {}: {}", icon, type_name, t),
                    None => format!("{} {}", icon, type_name),
                };
                let header_span = Span::styled(header_text, Style::default().fg(color).add_modifier(Modifier::BOLD));
                lines.push(virtual_line!([synthetic_span!(prefix.clone()), synthetic_span!(header_span)]));
            }

            // Body lines with type-colored border
            let body_lines: Vec<_> = nodes
                .iter()
                .enumerate()
                .flat_map(|(i, node)| {
                    let border_span = Span::raw("┃ ").fg(border_color);
                    let mut node_lines = render_node(
                        content.to_string(),
                        node,
                        max_width,
                        prefix.merge(border_span),
                        option,
                        symbols,
                        0,
                        None,
                    )
                    .lines;
                    if prefix.to_string().is_empty() && i != nodes.len().saturating_sub(1) {
                        let sep_border = Span::raw("┃ ").fg(border_color);
                        node_lines.extend([virtual_line!([synthetic_span!(sep_border)])]);
                    }
                    if prefix.to_string().is_empty() && i == nodes.len().saturating_sub(1) {
                        node_lines.extend([empty_virtual_line!()]);
                    }
                    node_lines
                })
                .collect();
            lines.extend(body_lines);
            lines
        },
    };

    VirtualBlock::new(&lines, source_range)
}

// FIXME: Use options struct or similar
#[allow(clippy::too_many_arguments)]
pub fn render_node<'a>(
    content: String,
    node: &ast::Node,
    max_width: usize,
    prefix: Span<'static>,
    option: &RenderStyle,
    symbols: &Symbols,
    list_depth: usize,
    syntect_ctx: Option<&SyntectContext>,
) -> VirtualBlock<'a> {
    use ast::Node::*;
    match node {
        Heading {
            level,
            text,
            source_range,
        } => heading(
            *level,
            &content,
            prefix,
            text,
            source_range,
            max_width,
            option,
            symbols,
        ),
        Paragraph { text, source_range } => paragraph(
            &content,
            prefix,
            text,
            source_range,
            max_width,
            option,
            symbols,
        ),
        CodeBlock {
            lang,
            text,
            source_range,
        } => code_block(
            &content,
            prefix,
            lang,
            syntect_ctx,
            text,
            source_range,
            max_width,
            option,
        ),
        List {
            nodes,
            source_range,
        } => list(
            &content,
            prefix,
            nodes,
            source_range,
            max_width,
            option,
            symbols,
            list_depth,
        ),
        Item {
            kind,
            nodes,
            source_range,
        } => item(
            &content,
            prefix,
            kind,
            nodes,
            source_range,
            max_width,
            option,
            symbols,
            list_depth,
        ),
        Task {
            kind,
            nodes,
            source_range,
        } => task(
            &content,
            prefix,
            kind,
            nodes,
            source_range,
            max_width,
            option,
            symbols,
            list_depth,
        ),
        BlockQuote {
            kind,
            title,
            nodes,
            source_range,
        } => block_quote(
            &content,
            prefix,
            kind,
            title,
            nodes,
            source_range,
            max_width,
            option,
            symbols,
        ),
    }
}

// Callout icon/color table - per-type styling
fn callout_style(kind: &ast::BlockQuoteKind) -> (Color, &'static str, &'static str, &'static str) {
    use ast::BlockQuoteKind::*;
    // (color, ascii_icon, unicode_icon, nerdfont_icon)
    match kind {
        // Standard GitHub Alert types (D-10)
        Note => (Color::Cyan, "[i]", "\u{2139}", "\u{f05a}"),
        Tip => (Color::Green, "[*]", "\u{2728}", "\u{f0eb}"),
        Important => (Color::Magenta, "[!]", "\u{2691}", "\u{f024}"),
        Warning => (Color::Yellow, "[!]", "\u{26a0}", "\u{f071}"),
        Caution => (Color::Red, "[x]", "\u{2716}", "\u{f00d}"),

        // ITS Theme - Citation/text (warm tones)
        Quote => (Color::Yellow, "[q]", "\u{275d}", "\u{f10d}"),
        Recite => (Color::LightYellow, "[r]", "\u{275e}", "\u{f10e}"),
        Aside => (Color::Cyan, "[>]", "\u{00bb}", "\u{f101}"),

        // ITS Theme - Structural/layout (cool tones)
        Cards => (Color::Blue, "[#]", "\u{25a3}", "\u{f0db}"),
        Grid => (Color::LightBlue, "[G]", "\u{25a6}", "\u{f00a}"),
        Column => (Color::LightCyan, "[|]", "\u{2503}", "\u{f0db}"),
        Kanban => (Color::Magenta, "[K]", "\u{25a4}", "\u{f24d}"),
        Timeline => (Color::LightMagenta, "[T]", "\u{25c6}", "\u{f017}"),

        // ITS Theme - Data/info (neutral)
        Infobox => (Color::White, "[I]", "\u{24d8}", "\u{f05a}"),
        Metadata => (Color::DarkGray, "[M]", "\u{2630}", "\u{f0c9}"),
        Statblocks => (Color::Gray, "[S]", "\u{2637}", "\u{f080}"),

        // ITS Theme - Task/action (green family)
        Checks => (Color::Green, "[v]", "\u{2611}", "\u{f046}"),
        Kith => (Color::LightGreen, "[k]", "\u{2619}", "\u{f0c0}"),

        // ITS Theme - Neutral
        Blank => (Color::DarkGray, "[ ]", "\u{25cb}", "\u{f10c}"),
        Caption => (Color::DarkGray, "[c]", "\u{2014}", "\u{f036}"),
    }
}

fn callout_type_name(kind: &ast::BlockQuoteKind) -> &'static str {
    use ast::BlockQuoteKind::*;
    match kind {
        Note => "NOTE",
        Tip => "TIP",
        Important => "IMPORTANT",
        Warning => "WARNING",
        Caution => "CAUTION",
        Aside => "ASIDE",
        Blank => "BLANK",
        Caption => "CAPTION",
        Cards => "CARDS",
        Checks => "CHECKS",
        Column => "COLUMN",
        Grid => "GRID",
        Infobox => "INFOBOX",
        Kanban => "KANBAN",
        Kith => "KITH",
        Metadata => "METADATA",
        Quote => "QUOTE",
        Recite => "RECITE",
        Statblocks => "STATBLOCKS",
        Timeline => "TIMELINE",
    }
}

fn callout_icon(kind: &ast::BlockQuoteKind, preset: &crate::config::Preset) -> &'static str {
    let (_, ascii, unicode, nerdfont) = callout_style(kind);
    match preset {
        crate::config::Preset::NerdFont => nerdfont,
        crate::config::Preset::Ascii => ascii,
        _ => unicode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Preset, Symbols};
    use crate::note_editor::rich_text::TextSegment;

    // Helper to convert VirtualBlock lines to snapshot-friendly string
    fn virtual_block_to_string(block: &VirtualBlock) -> String {
        block
            .lines
            .iter()
            .map(|line| {
                line.virtual_spans()
                    .iter()
                    .map(|span| match span {
                        VirtualSpan::Content(span, _) => span.content.to_string(),
                        VirtualSpan::Synthetic(span) => span.content.to_string(),
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // Test that callout_style returns the correct color for standard types
    #[test]
    fn test_callout_style_standard_colors() {
        let (note_color, _, _, _) = callout_style(&ast::BlockQuoteKind::Note);
        assert_eq!(note_color, Color::Cyan);

        let (tip_color, _, _, _) = callout_style(&ast::BlockQuoteKind::Tip);
        assert_eq!(tip_color, Color::Green);

        let (warning_color, _, _, _) = callout_style(&ast::BlockQuoteKind::Warning);
        assert_eq!(warning_color, Color::Yellow);

        let (caution_color, _, _, _) = callout_style(&ast::BlockQuoteKind::Caution);
        assert_eq!(caution_color, Color::Red);

        let (important_color, _, _, _) = callout_style(&ast::BlockQuoteKind::Important);
        assert_eq!(important_color, Color::Magenta);
    }

    // Test that callout_type_name returns uppercase strings
    #[test]
    fn test_callout_type_name_uppercase() {
        assert_eq!(callout_type_name(&ast::BlockQuoteKind::Note), "NOTE");
        assert_eq!(callout_type_name(&ast::BlockQuoteKind::Tip), "TIP");
        assert_eq!(
            callout_type_name(&ast::BlockQuoteKind::Important),
            "IMPORTANT"
        );
        assert_eq!(
            callout_type_name(&ast::BlockQuoteKind::Warning),
            "WARNING"
        );
        assert_eq!(callout_type_name(&ast::BlockQuoteKind::Caution), "CAUTION");
        assert_eq!(callout_type_name(&ast::BlockQuoteKind::Aside), "ASIDE");
        assert_eq!(callout_type_name(&ast::BlockQuoteKind::Kanban), "KANBAN");
        assert_eq!(
            callout_type_name(&ast::BlockQuoteKind::Statblocks),
            "STATBLOCKS"
        );
    }

    // Test that callout_icon returns different strings for different presets
    #[test]
    fn test_callout_icon_varies_by_preset() {
        let kind = ast::BlockQuoteKind::Note;
        let ascii_icon = callout_icon(&kind, &Preset::Ascii);
        let unicode_icon = callout_icon(&kind, &Preset::Unicode);
        let nerdfont_icon = callout_icon(&kind, &Preset::NerdFont);

        assert!(ascii_icon != unicode_icon);
        assert!(unicode_icon != nerdfont_icon);
        assert!(ascii_icon != nerdfont_icon);
    }

    // Insta snapshot tests for block_quote() render output
    #[test]
    fn snapshot_block_quote_note_callout() {
        let kind = Some(ast::BlockQuoteKind::Note);
        let title = None;
        let symbols = Symbols::unicode();
        let prefix = Span::raw("");
        let max_width = 80;
        let content = "Test body content.".to_string();
        let source_range = 0..20;

        let block = block_quote(
            &content,
            prefix,
            &kind,
            &title,
            &[ast::Node::Paragraph {
                text: RichText::from([TextSegment::plain("Test body content.")]),
                source_range: source_range.clone(),
            }],
            &source_range,
            max_width,
            &RenderStyle::Visual,
            &symbols,
        );

        insta::assert_snapshot!(virtual_block_to_string(&block));
    }

    #[test]
    fn snapshot_block_quote_warning_callout() {
        let kind = Some(ast::BlockQuoteKind::Warning);
        let title = None;
        let symbols = Symbols::unicode();
        let prefix = Span::raw("");
        let max_width = 80;
        let content = "Warning message here.".to_string();
        let source_range = 0..21;

        let block = block_quote(
            &content,
            prefix,
            &kind,
            &title,
            &[ast::Node::Paragraph {
                text: RichText::from([TextSegment::plain("Warning message here.")]),
                source_range: source_range.clone(),
            }],
            &source_range,
            max_width,
            &RenderStyle::Visual,
            &symbols,
        );

        insta::assert_snapshot!(virtual_block_to_string(&block));
    }

    #[test]
    fn snapshot_block_quote_aside_callout() {
        let kind = Some(ast::BlockQuoteKind::Aside);
        let title = None;
        let symbols = Symbols::unicode();
        let prefix = Span::raw("");
        let max_width = 80;
        let content = "This is an aside.".to_string();
        let source_range = 0..17;

        let block = block_quote(
            &content,
            prefix,
            &kind,
            &title,
            &[ast::Node::Paragraph {
                text: RichText::from([TextSegment::plain("This is an aside.")]),
                source_range: source_range.clone(),
            }],
            &source_range,
            max_width,
            &RenderStyle::Visual,
            &symbols,
        );

        insta::assert_snapshot!(virtual_block_to_string(&block));
    }

    #[test]
    fn snapshot_block_quote_note_with_title() {
        let kind = Some(ast::BlockQuoteKind::Note);
        let title = Some("My Title".to_string());
        let symbols = Symbols::unicode();
        let prefix = Span::raw("");
        let max_width = 80;
        let content = "Test body content.".to_string();
        let source_range = 0..20;

        let block = block_quote(
            &content,
            prefix,
            &kind,
            &title,
            &[ast::Node::Paragraph {
                text: RichText::from([TextSegment::plain("Test body content.")]),
                source_range: source_range.clone(),
            }],
            &source_range,
            max_width,
            &RenderStyle::Visual,
            &symbols,
        );

        insta::assert_snapshot!(virtual_block_to_string(&block));
    }

    #[test]
    fn snapshot_block_quote_plain() {
        let kind = None;
        let title = None;
        let symbols = Symbols::unicode();
        let prefix = Span::raw("");
        let max_width = 80;
        let content = "Plain blockquote text.".to_string();
        let source_range = 0..21;

        let block = block_quote(
            &content,
            prefix,
            &kind,
            &title,
            &[ast::Node::Paragraph {
                text: RichText::from([TextSegment::plain("Plain blockquote text.")]),
                source_range: source_range.clone(),
            }],
            &source_range,
            max_width,
            &RenderStyle::Visual,
            &symbols,
        );

        insta::assert_snapshot!(virtual_block_to_string(&block));
    }
}
