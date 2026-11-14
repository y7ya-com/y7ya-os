pub mod headless;
pub mod winit;

use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::utils::Transform;

use crate::state::Desk;

/// Creates the single output and shows it in the space.
pub fn add_output(desk: &mut Desk, name: &str, size: (i32, i32), scale: f64, transform: Transform) -> Output {
    let output = Output::new(
        name.to_string(),
        PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "y7ya-os".into(), model: name.into() },
    );
    let _global = output.create_global::<Desk>(&desk.display_handle);
    let mode = Mode { size: size.into(), refresh: 60_000 };
    output.change_current_state(Some(mode), Some(transform), Some(Scale::Fractional(scale)), Some((0, 0).into()));
    output.set_preferred(mode);
    desk.space.map_output(&output, (0, 0));
    desk.output = Some(output.clone());
    output
}
