use iced::widget::{container, mouse_area, text};
use iced::{Color, Element, Length};

use ai_partner_shared::{Message, Role};

use crate::app::AppEvent;

/// 渲染单条消息
pub fn view(msg: &Message) -> Element<'_, AppEvent> {
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
