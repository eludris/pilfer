use crate::AppContext;
use tui::{
    backend::Backend,
    layout::{Constraint, Corner, Direction, Layout},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub fn ui<B: Backend>(f: &mut Frame<B>, app: &AppContext) {
    let input_text: Vec<String> = app
        .input
        .split('\n') // handles empty line at the end
        .flat_map(|l| {
            if l.is_empty() {
                vec![String::new()]
            } else {
                let mut parts: Vec<String> = Vec::new();
                let mut l = l.to_string();
                let max_width = f.size().width - 2;
                loop {
                    if l.is_empty() {
                        break;
                    }
                    parts.push(
                        l.drain(
                            ..if l.len() > max_width as usize {
                                max_width as usize
                            } else {
                                l.len()
                            },
                        )
                        .collect::<String>(),
                    );
                }
                parts.into_iter().collect::<Vec<String>>()
            }
        })
        .rev()
        .take(((f.size().height - 2) / 3) as usize)
        .collect::<Vec<String>>()
        .into_iter()
        .rev()
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(1),
                Constraint::Length(
                    input_text.len() as u16 + if input_text.is_empty() { 3 } else { 2 },
                ),
            ]
            .as_ref(),
        )
        .split(f.size());

    let messages: Vec<ListItem> = app
        .messages
        .lock()
        .unwrap()
        .iter()
        .flat_map(|m| {
            // Seperates lines which are longer than the view width with newline characters
            // since it doesn't wrap sometimes for some reason
            m.0.to_string()
                .lines()
                .flat_map(|l| {
                    // Probably a newline
                    if l.is_empty() {
                        vec![ListItem::new("\n")]
                    } else {
                        let mut parts: Vec<String> = Vec::new();
                        let mut l = l.to_string();
                        loop {
                            if l.is_empty() {
                                break;
                            }
                            parts.push(
                                l.drain(
                                    ..if l.len() > (chunks[0].width - 2) as usize {
                                        (chunks[0].width - 2) as usize
                                    } else {
                                        l.len()
                                    },
                                )
                                .collect::<String>(),
                            );
                        }
                        parts
                            .into_iter()
                            .map(|l| ListItem::new(l).style(m.1))
                            .collect::<Vec<ListItem>>()
                    }
                })
                .collect::<Vec<ListItem>>()
        })
        .rev()
        .collect();

    let message_list = List::new(messages)
        .block(Block::default().borders(Borders::ALL).title("Messages"))
        .start_corner(Corner::BottomLeft);
    f.render_widget(message_list, chunks[0]);

    let text = input_text.join("\n");
    let input =
        Paragraph::new(text.as_ref()).block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(
        chunks[1].x + input_text.last().map(|l| l.width()).unwrap_or(0) as u16 + 1,
        chunks[1].y
            + if input_text.is_empty() {
                1
            } else {
                input_text.len() as u16
            },
    );
}
