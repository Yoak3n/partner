use iced::widget::{column, container, mouse_area, row, text, Space};
use iced::{Color, Element, Length};

use ai_partner_shared::SessionSummary;

use crate::app::AppEvent;

/// Session list item with preview, pin, archive, and delete buttons
pub fn view<'a>(session: &'a SessionSummary, is_active: bool, is_loading: bool) -> Element<'a, AppEvent> {
    let title = session
        .title
        .clone()
        .unwrap_or_else(|| session.id.chars().take(8).collect::<String>());

    let preview = session.first_message.as_ref().map(|m| {
        let t: String = m.chars().take(25).collect();
        if m.chars().count() > 25 {
            format!("{t}...")
        } else {
            t
        }
    });

    let label = if let Some(preview) = preview {
        column![
            text(title).size(14),
            text(preview)
                .size(11)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
        ]
    } else {
        column![text(title).size(14)]
    };

    let id = session.id.clone();
    let id_for_pin = session.id.clone();
    let id_for_archive = session.id.clone();
    let id_for_delete = session.id.clone();

    // Pin button - filled when pinned
    let pin_icon = if session.pinned { "📌" } else { "📍" };
    let pin_color = if session.pinned {
        Color::from_rgb(0.9, 0.7, 0.2)
    } else {
        Color::from_rgb(0.5, 0.5, 0.5)
    };

    // Archive button
    let archive_icon = if session.archived { "📦" } else { "📁" };
    let archive_color = if session.archived {
        Color::from_rgb(0.4, 0.6, 0.9)
    } else {
        Color::from_rgb(0.5, 0.5, 0.5)
    };

    let content = row![
        mouse_area(container(label.spacing(2)).width(Length::Fill).padding(8))
            .on_press(AppEvent::SwitchSession(id)),
        mouse_area(container(text(pin_icon).size(14).color(pin_color)).padding(4))
            .on_press(AppEvent::PinSession(id_for_pin)),
        mouse_area(container(text(archive_icon).size(14).color(archive_color)).padding(4))
            .on_press(AppEvent::ArchiveSession(id_for_archive)),
        mouse_area(container(text("×").size(14).color(Color::from_rgb(0.6, 0.3, 0.3))).padding(4))
            .on_press(AppEvent::DeleteSession(id_for_delete))
    ]
    .align_y(iced::Alignment::Center);

    let bg_color = if is_loading {
        Color::from_rgb(0.15, 0.15, 0.2)
    } else if is_active {
        Color::from_rgb(0.2, 0.2, 0.25)
    } else {
        Color::TRANSPARENT
    };

    container(content)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(bg_color.into()),
            ..Default::default()
        })
        .into()
}
