use iced::widget::{button, container, mouse_area, row, space, text};
use iced::{Element, Length};

use crate::app::AppEvent;

pub fn view() -> Element<'static, AppEvent> {
    let title = text("AI Partner").size(14);

    let close_btn = button(text("X").size(12))
        .on_press(AppEvent::WindowClose)
        .padding(iced::Padding::from([2, 8]));

    let maximize_btn = button(text("[ ]").size(12))
        .on_press(AppEvent::WindowMaximizeToggle)
        .padding(iced::Padding::from([2, 8]));

    let minimize_btn = button(text("-").size(12))
        .on_press(AppEvent::WindowMinimize)
        .padding(iced::Padding::from([2, 8]));

    let controls = row![minimize_btn, maximize_btn, close_btn].spacing(4);

    // 标题+空白区域可拖拽，按钮区域不可拖拽
    let drag_area = mouse_area(row![title, space::horizontal().width(Length::Fill)])
        .on_press(AppEvent::WindowDrag);

    let bar = row![drag_area, controls]
        .align_y(iced::Alignment::Center)
        .spacing(8);

    container(bar)
        .width(Length::Fill)
        .padding(iced::Padding::from([6, 12]))
        .into()
}
