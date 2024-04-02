struct WaylandWindow {
	
}

struct AppData;

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for AppData {
    fn event(
        _state: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<AppData>,
    ) {
        // When receiving events from the wl_registry, we are only interested in the
        // `global` event, which signals a new available global.
        // When receiving this event, we just print its characteristics in this example.
        if let wayland_client::protocol::wl_registry::Event::Global { name, interface, version } = event {
            println!("[{}] {} (v{})", name, interface, version);
        }
    }
}

impl WaylandWindow {
	pub fn new() -> Self {
		let conn = wayland_client::Connection::connect_to_env().unwrap();

		let display = conn.display();

		let mut event_queue = conn.new_event_queue();
		let qh = event_queue.handle();

		let registry = display.get_registry(&qh, ());

		// let compositor: wayland_client::protocol::wl_compositor::WlCompositor = registry.bind(1, 0, &qh, ());

		// let surface = compositor.create_surface(&qh, ());

		// let shell_surface = wayland_client::protocol::wl_shell::WlShell::get_shell_surface(&compositor, &surface, &qh);

		// shell_surface.set_title("Wayland Window".to_string());

		// shell_surface.set_toplevel();

		event_queue.roundtrip(&mut AppData).unwrap();

		Self {
			
		}
	}
}