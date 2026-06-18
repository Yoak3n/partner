use iced::widget::{column, container, mouse_area, text};
use iced::{Color, Element, Length};

use ai_partner_shared::{Message, Role, ToolCall};

use crate::app::AppEvent;

/// A display item that either is a regular message or a merged tool call + result.
pub enum DisplayItem {
    Message(Message),
    ToolCall {
        call: ToolCall,
        result: Option<String>,
    },
}

/// Pre-process messages to merge assistant tool_calls with their Tool results.
pub fn merge_messages(messages: &[Message]) -> Vec<DisplayItem> {
    // Collect tool results by call_id
    let mut tool_results: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for msg in messages {
        if msg.role == Role::Tool {
            if let Some(ref call_id) = msg.tool_call_id {
                tool_results.insert(call_id.clone(), msg.content.clone());
            }
        }
    }

    let mut items = Vec::new();
    for msg in messages {
        match msg.role {
            Role::Tool => {} // skip, merged into the tool call item
            Role::Assistant => {
                if let Some(ref calls) = msg.tool_calls {
                    // Push the assistant's text content first (if any)
                    if !msg.content.trim().is_empty() {
                        items.push(DisplayItem::Message(Message {
                            id: msg.id,
                            role: Role::Assistant,
                            content: msg.content.clone(),
                            timestamp: msg.timestamp,
                            tool_calls: None,
                            tool_call_id: None,
                        }));
                    }
                    // Then push each tool call as a collapsible item
                    for call in calls {
                        let result = tool_results.get(&call.id).cloned();
                        items.push(DisplayItem::ToolCall {
                            call: call.clone(),
                            result,
                        });
                    }
                } else {
                    items.push(DisplayItem::Message(msg.clone()));
                }
            }
            _ => items.push(DisplayItem::Message(msg.clone())),
        }
    }
    items
}

/// Render a regular message bubble.
pub fn view_message(msg: &Message) -> Element<'_, AppEvent> {
    let c = msg.content.clone();

    let bubble = match msg.role {
        Role::User => {
            let body = text(&msg.content).size(15);
            container(body)
                .padding(10)
                .max_width(600)
                .style(container::rounded_box)
        }
        Role::Assistant => {
            let body = text(&msg.content).size(15);
            container(body)
                .padding(10)
                .max_width(600)
                .style(container::rounded_box)
        }
        Role::System => {
            let body = text(&msg.content).size(12).color(Color::from_rgb(0.6, 0.6, 0.6));
            container(body).padding(8)
        }
        Role::Tool => {
            let body = text(&msg.content).size(12).color(Color::from_rgb(0.5, 0.8, 0.5));
            container(body).padding(8)
        }
    };

    let align = match msg.role {
        Role::User => iced::Alignment::End,
        _ => iced::Alignment::Start,
    };

    mouse_area(
        container(bubble)
            .width(Length::Fill)
            .align_x(align)
            .padding([0, 12]),
    )
    .on_press(AppEvent::CopyText(c))
    .into()
}

/// Render a collapsible tool call item.
pub fn view_tool_call<'a>(call: &'a ToolCall, result: &'a Option<String>, expanded: bool) -> Element<'a, AppEvent> {
    let status_icon = if result.is_some() { "▸" } else { "…" };
    let header_text = format!("{} {}", status_icon, call.name);
    let header = text(header_text)
        .size(13)
        .color(Color::from_rgb(0.6, 0.8, 1.0));

    let mut content = column![].spacing(4);
    content = content.push(
        container(header)
            .padding(iced::Padding::from([6, 10]))
            .width(Length::Fill),
    );

    if expanded {
        // Show arguments
        let args_str = call.arguments.to_string();
        let args_display: String = args_str.chars().take(500).collect();
        let args_text = if args_str.len() > 500 {
            format!("{args_display}...")
        } else {
            args_display
        };
        content = content.push(
            container(
                text(format!("args: {args_text}"))
                    .size(12)
                    .color(Color::from_rgb(0.7, 0.7, 0.7)),
            )
            .padding(iced::Padding::from([0, 10])),
        );

        // Show result
        if let Some(res) = result {
            let preview: String = res.chars().take(800).collect();
            let result_text = if res.len() > 800 {
                format!("{preview}...")
            } else {
                preview.clone()
            };
            content = content.push(
                container(
                    text(result_text)
                        .size(12)
                        .color(Color::from_rgb(0.5, 0.8, 0.5)),
                )
                .padding(iced::Padding::from([0, 10])),
            );
        }
    }

    let call_id = call.id.clone();
    mouse_area(
        container(content)
            .width(Length::Fill)
            .padding([0, 12])
            .style(|_| iced::widget::container::Style {
                background: Some(Color::from_rgba(0.15, 0.2, 0.3, 0.3).into()),
                border: iced::border::rounded(6),
                ..Default::default()
            }),
    )
    .on_press(AppEvent::ToggleToolCall(call_id))
    .into()
}
