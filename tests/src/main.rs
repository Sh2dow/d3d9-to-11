#![cfg(windows)]

use std::time::{Duration, Instant};
use winit::{
    event::{Event, WindowEvent, VirtualKeyCode},
    event_loop::{ControlFlow, EventLoop},
    platform::windows::WindowExtWindows,
    window::Window,
};

mod context;
mod device;

fn handle_event(event: &WindowEvent, should_close: &mut bool) {
    match event {
        WindowEvent::CloseRequested => *should_close = true,
        WindowEvent::KeyboardInput { input, .. } => {
            if let Some(VirtualKeyCode::Escape) = input.virtual_keycode {
                *should_close = true;
            }
        }
        _ => {}
    }
}

fn main() {
    let event_loop = EventLoop::new();
    let window = Window::new(&event_loop).expect("Failed to create window");

    let ctx = context::create_context();
    context::run_tests(&ctx);

    let hwnd = window.hwnd() as *mut _;
    let mut device = device::Device::new(&ctx, hwnd);
    device.run_tests();

    let mut should_close = false;
    const MAX_TIME: Duration = Duration::from_secs(5);
    let start = Instant::now();

    // Switch to the modern `run` event loop, as `poll_events` is deprecated.
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent { event, .. } = event {
            handle_event(&event, &mut should_close);
        }

        // Close the application after MAX_TIME or if the window is closed.
        if start.elapsed() > MAX_TIME || should_close {
            *control_flow = ControlFlow::Exit;
        }
    });
}
