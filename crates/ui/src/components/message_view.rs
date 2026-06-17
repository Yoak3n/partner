use iced::widget::{column, container, text};
use iced::{Color, Element, Length};

use ai_partner_shared::{Message, Role};

use crate::app::AppEvent;

/// 渲染单条消息，根据角色使用不同样式
pub fn view(msg: &Message) -> Element<'static, AppEvent> {
    match msg.role {
        Role::User => user_message(&msg.content),
        Role::Assistant => assistant_message(&msg.content),
        Role::System => system_message(&msg.content),
        Role::Tool => tool_message(&msg.content),
    }
}

/// 渲染流式输出中的 assistant 消息
pub fn streaming_view(content: &str) -> Element<'static, AppEvent> {
    let label = text("AI").size(11).color(Color::from_rgb(0.6, 0.8, 1.0));
    let body = text(content.to_owned()).size(15);

    let bubble = container(column![label, body].spacing(4))
        .padding(12)
        .max_width(700)
        .width(Length::Shrink)
        .style(container::rounded_box);

    container(bubble)
        .width(Length::Fill)
        .padding(iced::Padding::from([0, 16]))
        .into()
}

/// 用户消息 — 右对齐，蓝色气泡
fn user_message(content: &str) -> Element<'static, AppEvent> {
    let label = text("You").size(11).color(Color::from_rgb(0.7, 0.85, 1.0));
    let body = text(content.to_owned()).size(15);

    let bubble = container(column![label, body].spacing(4))
        .padding(12)
        .max_width(700)
        .width(Length::Shrink)
        .style(container::rounded_box);

    container(bubble)
        .width(Length::Fill)
        .align_x(iced::Alignment::End)
        .padding(iced::Padding::from([0, 16]))
        .into()
}

/// AI 回复 — 左对齐，深色气泡
fn assistant_message(content: &str) -> Element<'static, AppEvent> {
    let label = text("AI").size(11).color(Color::from_rgb(0.6, 0.8, 1.0));
    let body = text(content.to_owned()).size(15);

    let bubble = container(column![label, body].spacing(4))
        .padding(12)
        .max_width(700)
        .width(Length::Shrink)
        .style(container::rounded_box);

    container(bubble)
        .width(Length::Fill)
        .padding(iced::Padding::from([0, 16]))
        .into()
}

/// 系统消息 — 居中，小字灰色
fn system_message(content: &str) -> Element<'static, AppEvent> {
    let label = text("System").size(10).color(Color::from_rgb(0.5, 0.5, 0.5));
    let body = text(content.to_owned()).size(13).color(Color::from_rgb(0.6, 0.6, 0.6));

    let bubble = container(column![label, body].spacing(2))
        .padding(iced::Padding::from([6, 12]))
        .style(container::rounded_box);

    container(bubble)
        .width(Length::Fill)
        .align_x(iced::Alignment::Center)
        .padding(iced::Padding::from([0, 32]))
        .into()
}

/// 工具消息 — 左对齐，等宽字体风格
fn tool_message(content: &str) -> Element<'static, AppEvent> {
    let label = text("Tool").size(10).color(Color::from_rgb(0.8, 0.7, 0.4));
    let body = text(content.to_owned()).size(13).color(Color::from_rgb(0.75, 0.75, 0.75));

    let bubble = container(column![label, body].spacing(2))
        .padding(iced::Padding::from([6, 12]))
        .max_width(700)
        .width(Length::Shrink)
        .style(container::rounded_box);

    container(bubble)
        .width(Length::Fill)
        .padding(iced::Padding::from([0, 16]))
        .into()
}
