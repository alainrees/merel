//! Release notes as the update window shows them: the little Markdown our notes use (headings,
//! bullets, bold) drawn as text. Links show only their words and pictures are left out, so
//! nothing is fetched or opened from the notes.

use egui::text::{LayoutJob, TextFormat};
use egui::{FontFamily, FontId, Label, Ui};
use merel_model::rich::Face;

use crate::theme::Tokens;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Bullet(Vec<Span>),
    Paragraph(Vec<Span>),
}

/// A run of text, bold or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
}

/// The blocks of `notes`. Lines that follow each other form one paragraph or bullet.
pub fn parse(notes: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Option<(bool, String)> = None;
    let flush = |open: &mut Option<(bool, String)>, blocks: &mut Vec<Block>| {
        if let Some((bullet, text)) = open.take() {
            let spans = spans(&text);
            if !spans.is_empty() {
                blocks.push(if bullet { Block::Bullet(spans) } else { Block::Paragraph(spans) });
            }
        }
    };
    for line in notes.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut open, &mut blocks);
        } else if trimmed.starts_with('#') {
            flush(&mut open, &mut blocks);
            let title: String = spans(trimmed.trim_start_matches('#')).into_iter().map(|s| s.text).collect();
            if !title.is_empty() {
                blocks.push(Block::Heading(title));
            }
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ ")) {
            flush(&mut open, &mut blocks);
            open = Some((true, rest.to_owned()));
        } else {
            match &mut open {
                Some((_, text)) => {
                    text.push(' ');
                    text.push_str(trimmed);
                }
                None => open = Some((false, trimmed.to_owned())),
            }
        }
    }
    flush(&mut open, &mut blocks);
    blocks
}

/// `text` split into plain and bold runs, with the Markdown marks taken out.
fn spans(text: &str) -> Vec<Span> {
    let text = plain_links(text);
    let mut out: Vec<Span> = Vec::new();
    let mut bold = false;
    for (i, part) in text.split("**").enumerate() {
        if i > 0 {
            bold = !bold;
        }
        let part = part.replace('`', "");
        if part.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.bold == bold => last.text.push_str(&part),
            _ => out.push(Span { text: part, bold }),
        }
    }
    if let Some(first) = out.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = out.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    out.retain(|s| !s.text.is_empty());
    out
}

/// `[words](address)` becomes its words and `![words](address)` disappears.
fn plain_links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let picture = rest[..start].ends_with('!');
        let Some(close) = rest[start..].find("](").map(|i| start + i) else { break };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else { break };
        out.push_str(&rest[..if picture { start - 1 } else { start }]);
        if !picture {
            out.push_str(&rest[start + 1..close]);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Draws `notes`, wrapped to the width there is.
pub fn show(ui: &mut Ui, t: &Tokens, notes: &str) {
    let bold_family = {
        let (family, _) = crate::fonts::family(Face::Sans, true, false);
        let loaded = ui.ctx().fonts(|f| f.definitions().families.contains_key(&family));
        if loaded { family } else { FontFamily::Proportional }
    };
    let format = |size: f32, bold: bool| TextFormat {
        font_id: FontId::new(size, if bold { bold_family.clone() } else { FontFamily::Proportional }),
        color: t.text,
        ..Default::default()
    };
    for (i, block) in parse(notes).iter().enumerate() {
        if i > 0 {
            ui.add_space(6.0);
        }
        match block {
            Block::Heading(title) => {
                ui.add(Label::new(LayoutJob::single_section(title.clone(), format(15.0, true))).wrap());
            }
            Block::Paragraph(spans) => {
                let mut job = LayoutJob::default();
                for span in spans {
                    job.append(&span.text, 0.0, format(14.0, span.bold));
                }
                ui.add(Label::new(job).wrap());
            }
            Block::Bullet(spans) => {
                ui.horizontal_top(|ui| {
                    ui.label(LayoutJob::single_section("•".to_owned(), format(14.0, false)));
                    let mut job = LayoutJob::default();
                    for span in spans {
                        job.append(&span.text, 0.0, format(14.0, span.bold));
                    }
                    ui.add(Label::new(job).wrap());
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span { text: text.into(), bold: false }
    }

    fn bold(text: &str) -> Span {
        Span { text: text.into(), bold: true }
    }

    #[test]
    fn our_notes_lose_their_marks() {
        let notes = "## Nieuw in Merel 0.1.5\n\n- **Menubalk zoals Outlook.** Bestand en Help\nstaan in de menubalk.\n- Tweede punt.\n\nKies **Bijwerken** in het venster.";
        assert_eq!(
            parse(notes),
            [
                Block::Heading("Nieuw in Merel 0.1.5".into()),
                Block::Bullet(vec![bold("Menubalk zoals Outlook."), plain(" Bestand en Help staan in de menubalk.")]),
                Block::Bullet(vec![plain("Tweede punt.")]),
                Block::Paragraph(vec![plain("Kies "), bold("Bijwerken"), plain(" in het venster.")]),
            ]
        );
    }

    #[test]
    fn links_keep_their_words_and_pictures_go() {
        assert_eq!(plain_links("Zie [de pagina](https://example.org) ![logo](https://x/y.png)nu."), "Zie de pagina nu.");
        assert_eq!(plain_links("Half [link zonder einde"), "Half [link zonder einde");
    }

    #[test]
    fn a_cut_off_bold_mark_does_no_harm() {
        assert_eq!(parse("Tekst **half…"), [Block::Paragraph(vec![plain("Tekst "), bold("half…")])]);
    }
}
