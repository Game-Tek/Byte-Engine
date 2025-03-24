use byte_engine::{application::{Application, Parameter}, core::spawn_as_child, rendering::triangle::Triangle};

#[test]
fn triangle() {
    // let mut app = byte_engine::application::GraphicsApplication::new("Triangle Smoke Test", &[Parameter::new("resources-path", "../../resources"), Parameter::new("assets-path", "../../assets"), Parameter::new("kill-after", "60")]);
    let mut app = byte_engine::application::GraphicsApplication::new("Triangle Smoke Test", &[Parameter::new("kill-after", "60")]);

    let space_handle = app.get_root_space_handle();

    spawn_as_child(space_handle.clone(), Triangle::new());

	app.do_loop();
}
