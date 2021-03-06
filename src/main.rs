pub mod compute;
pub mod engine;
pub mod gl;
pub mod imgui_vulkano_renderer;
pub mod material;
mod mesh;
pub mod physics;
pub mod player;
pub mod rendering;
pub mod sprite;
pub mod texture;
mod tilemap;
pub mod transform;
pub mod uniform;

pub use bevy_ecs::prelude as ecs;
use bevy_ecs::schedule::Stage;
use vulkano::pipeline::Pipeline;

use std::sync::{Arc, Mutex};

use imgui_vulkano_renderer::ImGuiRenderer;

use vulkano::command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage, SubpassContents};

use vulkano::device::physical::{PhysicalDevice, PhysicalDeviceType, QueueFamily};
use vulkano::device::DeviceExtensions;

use vulkano::instance::Instance;

use vulkano::swapchain::{self, AcquireError, Surface};
use vulkano::sync::{self, FlushError, GpuFuture};

use vulkano_win::VkSurfaceBuild;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, Event, MouseButton, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};
mod clipboard;
use imgui::{self};

use crate::mesh::Mesh;
use crate::texture::Texture;

fn get_physical<'a>(
    instance: &'a Arc<Instance>,
    device_extensions: DeviceExtensions,
    surface: &Surface<Window>,
) -> (PhysicalDevice<'a>, QueueFamily<'a>) {
    // pick the best physical device and queue1
    PhysicalDevice::enumerate(instance)
        .filter(|&p| p.supported_extensions().is_superset_of(&device_extensions))
        .filter_map(|p| {
            p.queue_families()
                // Find the first first queue family that is suitable.
                // If none is found, `None` is returned to `filter_map`,
                // which disqualifies this physical device.
                .find(|&q| q.supports_graphics() && q.supports_surface(surface).unwrap_or(false))
                .map(|q| (p, q))
        })
        .min_by_key(|(p, _)| match p.properties().device_type {
            PhysicalDeviceType::DiscreteGpu => 0,
            PhysicalDeviceType::IntegratedGpu => 1,
            PhysicalDeviceType::VirtualGpu => 2,
            PhysicalDeviceType::Cpu => 3,
            PhysicalDeviceType::Other => 4,
        })
        .expect("no device available")
}

pub struct Time {
    t: f32,
    dt: f32,
}
impl Time {
    pub fn progress(&mut self) {
        self.t += self.dt;
    }
}

pub struct InputEvent {
    pub keycode: VirtualKeyCode,
    pub state: ElementState,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, ecs:: StageLabel)]
enum SystemTrigger {
    OnUpdate,
    OnKeyboardInput,
}

fn main() -> ! {
    println!("Hello, world!");
    let instance = engine::get_instance();
    let event_loop = EventLoop::new(); // ignore this for now
    let surface = WindowBuilder::new()
        .build_vk_surface(&event_loop, instance.clone())
        .unwrap();

    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::none()
    };

    //In the previous section we created an instance and chose a physical device from this instance.
    let (physical_device, graphics_queue) = get_physical(&instance, device_extensions, &surface);

    let mut engine = engine::Engine::init(
        &physical_device,
        &graphics_queue,
        surface,
        &device_extensions,
    );

    let square = <dyn Mesh>::create_unit_square(engine.queue());

    //let vs = vs::load(device.clone()).unwrap();
    //let vs_texture = vs_texture::load(engine.device()).unwrap();
    //    let fs = fs::load(device.clone()).unwrap();
    //let fs_texture = fs_texture::load(engine.device()).unwrap();

    //let mut tile_positions = [[1f32, 1f32], [1f32, 1f32], [1f32, 1f32]];

    // let uniform_data_buffer =
    //     CpuAccessibleBuffer::from_iter(engine.device(), BufferUsage::all(), false, tile_positions)
    //         .expect("failed to create buffer");

    // let mut command_buffers = gl::get_draw_command_buffers(
    //     device.clone(),
    //     queue.clone(),
    //     pipeline.clone(),
    //     &framebuffers,
    //     vertex_buffer.clone(),
    //     index_buffer.clone(),
    //     uniform_set.clone(),
    // );

    let mut window_resized = false;
    let mut recreate_swapchain = false;

    let mut t = 0f32;

    let mut transform = uniform::Transformations::new(engine.device());

    //let w_s = transform.transform();

    let screen_size = engine.surface().window().inner_size();

    //let aspect = screen_size.width as f32 / screen_size.height as f32;

    transform.update_buffer();

    //let cobblestone = Texture::load("assets/cobblestone.png", &engine);

    // let mat_texture = engine.create_material(
    //     vs_texture,
    //     fs_texture,
    //     [
    //         // 0 is the binding in GLSL when we use this set
    //         WriteDescriptorSet::buffer(0, transform.get_buffer()),
    //         WriteDescriptorSet::buffer(1, uniform_data_buffer.clone()),
    //         cobblestone.describe(3),
    //     ],
    // );

    let desert_sprite_sheet = Texture::load("assets/tileset.png", &engine);

    let moleman_sprite_sheet = Texture::load("assets/moleman.png", &engine);

    let mut dragging = false;

    let mut last_mouse_pos: Option<PhysicalPosition<f64>> = None;
    let mut was_dragging = false;

    // Example with default allocator
    // IMGUI BS
    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);

    if let Some(backend) = clipboard::init() {
        imgui.set_clipboard_backend(backend);
    } else {
        eprintln!("Failed to initialize clipboard");
    }

    let mut platform = imgui_winit_support::WinitPlatform::init(&mut imgui);
    platform.attach_window(
        imgui.io_mut(),
        engine.surface().window(),
        imgui_winit_support::HiDpiMode::Rounded,
    );

    let hidpi_factor = platform.hidpi_factor();
    let font_size = (13.0 * hidpi_factor) as f32;
    imgui
        .fonts()
        .add_font(&[imgui::FontSource::DefaultFontData {
            config: Some(imgui::FontConfig {
                size_pixels: font_size,
                ..imgui::FontConfig::default()
            }),
        }]);

    imgui.io_mut().font_global_scale = (1.0 / hidpi_factor) as f32;

    let format = engine.swapchain().swapchain().image_format();

    let mut renderer = ImGuiRenderer::init(&mut imgui, engine.device(), engine.queue(), format)
        .expect("Failed to initialize renderer");

    //create the tilemap for the desert tile map then create it's material
    let config = Arc::new(Mutex::new(tilemap::TilemapSpriteConfig::new_or_load(
        "assets/tileset.png.tileset.json",
        16,
        8,
    )));

    let desert =
        tilemap::TilemapRenderer::new(config.clone(), desert_sprite_sheet.clone(), &engine);

    let desert_mat = desert.create_material(&mut engine, &transform);

    let mut config_editor = tilemap::sprite_config_editor::TilemapSpriteConfigEditor::new(
        &mut renderer,
        config.clone(),
        desert_sprite_sheet.clone(),
    );

    let mole_mat = sprite::create_sprite_material(&mut engine, &moleman_sprite_sheet, &transform);

    let mole_sprite = Arc::new(sprite::Sprite {
        grid_width: 3,
        grid_height: 1,
        tile_width: 32,
        tile_height: 32,
    });

    let mole_sprite_data = sprite::SpriteData {
        sprite: mole_sprite.clone(),
        tile_x: 0,
        tile_y: 0,
    };

    //    let m = engine.get_material(&desert_mat);

    // let mut desert_cmd_builder = engine.create_secondary(
    //     CommandBufferUsage::MultipleSubmit,
    //     engine.render_pass().render_pass().first_subpass(),
    // );

    // let desert_cmd = Arc::new(desert_cmd_builder.build().unwrap());

    //ECS----------------------------------------------------------

    // Create a new empty World to hold our Entities and Components
    let mut world = ecs::World::new();

    // Spawn an entity with Position and Velocity components
    let mut inspecting = world
        .spawn()
        .insert(desert)
        .insert(rendering::Renderer {
            material: desert_mat,
        })
        .id();

    world
        .spawn()
        .insert(mole_sprite_data)
        .insert(rendering::Renderer { material: mole_mat })
        .insert(transform::Position(0.0, 0.0))
        .insert(physics::Velocity(0.0, 0.0))
        .insert(player::Player { speed: 1.0 });

    world.insert_resource(Time { t: 0.0, dt: 0.1 });

    // Create a new Schedule, which defines an execution strategy for Systems
    let mut schedule = ecs::Schedule::default();
    // Add a Stage to our schedule. Each Stage in a schedule runs all of its systems
    // before moving on to the next Stage

    schedule.add_stage(
        SystemTrigger::OnUpdate,
        ecs::SystemStage::parallel()
            .with_system(tilemap::tilemap_on_update)
            .with_system(transform::bobble_on_update)
            .with_system(physics::on_update),
    );

    schedule.add_stage(
        SystemTrigger::OnKeyboardInput,
        ecs::SystemStage::parallel().with_system(player::on_keyboard_input),
    );

    // MAIN LOOP ---------------------------------------------------------

    event_loop.run(move |event, _, control_flow| {
        platform.handle_event(imgui.io_mut(), engine.surface().window(), &event);
        // Trigger system events

        match event {
            Event::MainEventsCleared => {
                world.get_resource_mut::<Time>().unwrap().progress();

                schedule
                    .get_stage_mut::<ecs::SystemStage>(&SystemTrigger::OnUpdate)
                    .unwrap()
                    .run(&mut world);
            }
            Event::WindowEvent {
                event: ref window_event,
                ..
            } => match window_event {
                WindowEvent::Resized(_) => (),
                WindowEvent::Moved(_) => (),
                WindowEvent::CloseRequested => (),
                WindowEvent::Destroyed => (),
                WindowEvent::DroppedFile(_) => (),
                WindowEvent::HoveredFile(_) => (),
                WindowEvent::HoveredFileCancelled => (),
                WindowEvent::ReceivedCharacter(_) => (),
                WindowEvent::Focused(_) => (),
                WindowEvent::KeyboardInput {
                    device_id,
                    input,
                    is_synthetic,
                } => {
                    if let Some(v) = input.virtual_keycode {
                        world.insert_resource(InputEvent {
                            keycode: v,
                            state: input.state,
                        });

                        schedule
                            .get_stage_mut::<ecs::SystemStage>(&SystemTrigger::OnKeyboardInput)
                            .map(|s| s.run(&mut world));
                    }
                }
                WindowEvent::ModifiersChanged(_) => (),
                WindowEvent::CursorMoved {
                    device_id,
                    position,
                    ..
                } => (),
                WindowEvent::CursorEntered { device_id } => (),
                WindowEvent::CursorLeft { device_id } => (),
                WindowEvent::MouseWheel {
                    device_id,
                    delta,
                    phase,
                    ..
                } => (),
                WindowEvent::MouseInput {
                    device_id,
                    state,
                    button,
                    ..
                } => (),
                WindowEvent::TouchpadPressure {
                    device_id,
                    pressure,
                    stage,
                } => (),
                WindowEvent::AxisMotion {
                    device_id,
                    axis,
                    value,
                } => (),
                WindowEvent::Touch(_) => (),
                WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                } => (),
                WindowEvent::ThemeChanged(_) => (),
            },
            _ => (),
        };

        // Manage window
        match event {
            Event::RedrawEventsCleared => {
                if window_resized || recreate_swapchain {
                    recreate_swapchain = false;

                    // recreate the swapchain. this *may* result in a new sized image, in this case also update the viewport
                    let new_dimensions = match engine.recreate_swapchain() {
                        Err(()) => return,
                        Ok(new_dimensions) => new_dimensions,
                    };

                    if window_resized {
                        window_resized = false;
                        //println!("Updating window size");

                        engine.update_viewport(new_dimensions.into());

                        let aspect = new_dimensions.height as f32 / new_dimensions.width as f32;
                        {
                            let m = transform.transform();
                            //FIXME:
                            //changing the x scale gives a more natural looking scaling, but would be better if entire thing zoomed out

                            *m = glm::mat4(
                                0.1 * aspect,
                                0.,
                                0.,
                                0., //
                                0.,
                                -0.1,
                                0.,
                                0., //
                                0.,
                                0.,
                                1.,
                                0., //
                                0.,
                                0.,
                                0.,
                                1., //
                            );
                        }
                        transform.update_buffer();

                        // command_buffers = gl::get_draw_command_buffers(
                        //     device.clone(),
                        //     queue.clone(),
                        //     pipeline.clone(),
                        //     &new_framebuffers,
                        //     vertex_buffer.clone(),
                        //     index_buffer.clone(),
                        //     uniform_set.clone(),
                        // );
                    }
                }
            }

            Event::MainEventsCleared => {
                //To actually start drawing, the first thing that we need to do is to acquire an image to draw:
                let (image_i, suboptimal, acquire_future) =
                    match swapchain::acquire_next_image(engine.swapchain().swapchain(), None) {
                        Ok(r) => r,
                        Err(AcquireError::OutOfDate) => {
                            recreate_swapchain = true;
                            return;
                        }
                        Err(e) => panic!("Failed to acquire next image: {:?}", e),
                    };

                if suboptimal {
                    recreate_swapchain = true;
                }

                platform
                    .prepare_frame(imgui.io_mut(), engine.surface().window())
                    .unwrap();

                let ui = imgui.frame();

                imgui::Window::new("Tilemap Data Editor")
                    .size([300.0, 110.0], imgui::Condition::FirstUseEver)
                    .build(&ui, || config_editor.run(&ui));
                //get entity will not panic if no entity present
                if let Some(i) = world.get_entity(inspecting) {
                    if let Some(tilemap) = i.get::<tilemap::TilemapRenderer>() {
                        imgui::Window::new("Tilemap - desert")
                            .size([200.0, 200.0], imgui::Condition::FirstUseEver)
                            .build(&ui, || {
                                let [wx, wy] = ui.window_pos();

                                let l = ui.get_window_draw_list();
                                let w = 15.0;
                                let h = 15.0;
                                for x in 0..16 {
                                    for y in 0..16 {
                                        if let tilemap::Tile::Filled(o) = *tilemap.tile(x, y) {
                                            fn col(e: bool) -> u8 {
                                                if e {
                                                    255
                                                } else {
                                                    0
                                                }
                                            }
                                            //North and south are opposite of what we expect because of flipped y direction
                                            let col_upr_left = imgui::ImColor32::from_rgb(
                                                col(o.contains(tilemap::Orientation::N)),
                                                col(o.contains(tilemap::Orientation::W)),
                                                col(o.contains(tilemap::Orientation::NW)),
                                            );

                                            let col_upr_right = imgui::ImColor32::from_rgb(
                                                col(o.contains(tilemap::Orientation::N)),
                                                col(o.contains(tilemap::Orientation::E)),
                                                col(o.contains(tilemap::Orientation::NE)),
                                            );

                                            let col_bot_left = imgui::ImColor32::from_rgb(
                                                col(o.contains(tilemap::Orientation::S)),
                                                col(o.contains(tilemap::Orientation::W)),
                                                col(o.contains(tilemap::Orientation::SW)),
                                            );

                                            let col_bot_right = imgui::ImColor32::from_rgb(
                                                col(o.contains(tilemap::Orientation::S)),
                                                col(o.contains(tilemap::Orientation::E)),
                                                col(o.contains(tilemap::Orientation::SE)),
                                            );

                                            //Y coordinates need to be flipped as we draw from top to bottom,
                                            //and so y coordinates increase as we decrease in window space
                                            l.add_rect_filled_multicolor(
                                                [
                                                    10.0 + wx + w * x as f32,
                                                    30.0 + wy + h * (16 - y) as f32,
                                                ],
                                                [
                                                    10.0 + wx + w * (x + 1) as f32,
                                                    30.0 + wy + h * (16 - (y + 1)) as f32,
                                                ],
                                                col_upr_left,
                                                col_upr_right,
                                                col_bot_right,
                                                col_bot_left,
                                            );
                                        }
                                    }
                                }
                            });
                    }
                }

                platform.prepare_render(&ui, engine.surface().window());

                let draw_data = ui.render();

                let framebuffer = &engine.render_pass().get_frame(image_i);

                let cmd_buffer = {
                    //build the command buffer
                    let mut builder = AutoCommandBufferBuilder::primary(
                        engine.device(),
                        engine.queue().family(),
                        CommandBufferUsage::OneTimeSubmit, // don't forget to write the correct buffer usage
                    )
                    .unwrap();

                    // begin render pass
                    builder
                        .begin_render_pass(
                            framebuffer.clone(),
                            SubpassContents::Inline,
                            vec![[0.0, 0.0, 0.0, 1.0].into()],
                        )
                        .unwrap();

                    //now, attempt to render the desert map, which has to be in a different subpass

                    //builder.execute_commands(desert_cmd.clone()).unwrap();

                    //    builder.next_subpass(SubpassContents::Inline).unwrap();

                    // engine
                    //     .get_material(&mat_texture)
                    //     .draw(&mut builder, &*square, 3);

                    //render pass started, can now issue draw instructions

                    for (renderer, tilemap) in world
                        .query::<(&rendering::Renderer, &tilemap::TilemapRenderer)>()
                        .iter(&mut world)
                    {
                        let e = engine.get_material(&renderer.material);
                        e.bind(&mut builder, &*square);
                        e.draw(&mut builder, &*square, tilemap.instance_count());
                    }

                    for (renderer, sprite_data, pos) in world
                        .query::<(
                            &rendering::Renderer,
                            &sprite::SpriteData,
                            &transform::Position,
                        )>()
                        .iter(&mut world)
                    {
                        let e = engine.get_material(&renderer.material);

                        e.bind(&mut builder, &*square);

                        builder.push_constants(
                            e.pipeline.layout().clone(),
                            0,
                            sprite_data.get_push_constants(pos),
                        );

                        e.draw(&mut builder, &*square, 1);
                    }

                    renderer
                        .draw_commands(
                            &mut builder,
                            engine.queue(),
                            engine.viewport().dimensions,
                            draw_data,
                        )
                        .unwrap();

                    //finish off
                    builder.end_render_pass().unwrap();

                    //return the created command buffer
                    builder.build().unwrap()
                };

                // let mut i = 0f32;
                // for p in &mut tile_positions[1..] {
                //     *p = [(t + i).cos(), (t + i).sin()];
                //     i += 1.;
                // }

                // {
                //     //update buffer data
                //     let mut w = uniform_data_buffer.write().expect("failed to write buffer");

                //     for (i, p) in tile_positions.iter().enumerate() {
                //         w[i] = *p;
                //     }
                // }

                //create the future to execute our command buffer
                let cmd_future = sync::now(engine.device())
                    .join(acquire_future)
                    .then_execute(engine.queue(), cmd_buffer)
                    .unwrap();

                //fence is from GPU -> CPU sync, semaphore is GPU to GPU.

                let execution = cmd_future
                    .then_swapchain_present(
                        engine.queue().clone(),
                        engine.swapchain().swapchain(),
                        image_i,
                    )
                    .then_signal_fence_and_flush();

                match execution {
                    Ok(future) => {
                        future.wait(None).unwrap(); // wait for the GPU to finish
                    }
                    Err(FlushError::OutOfDate) => {
                        recreate_swapchain = true;
                    }
                    Err(e) => {
                        println!("Failed to flush future: {:?}", e);
                    }
                }

                t += 0.02;
            }

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } if !imgui.io().want_capture_mouse => {
                if dragging && was_dragging {
                    if let Some(last_pos) = last_mouse_pos {
                        let diff_x =
                            ((position.x - last_pos.x) as f32) * 2. / screen_size.width as f32;
                        let diff_y =
                            ((position.y - last_pos.y) as f32) * 2. / screen_size.height as f32;

                        transform.transform().c0.w += diff_x;
                        transform.transform().c1.w += diff_y;

                        transform.update_buffer();
                    }
                }

                last_mouse_pos = Some(position);
                was_dragging = dragging;
            }

            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state,
                        button: MouseButton::Left,
                        ..
                    },
                ..
            } if !imgui.io().want_capture_mouse => {
                dragging = state == ElementState::Pressed;
            }

            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state,
                        button: MouseButton::Right,
                        ..
                    },
                ..
            } if !imgui.io().want_capture_mouse && state == ElementState::Pressed => {
                //alter the tilemap;
                //first get mouse pos in tilemap, then alter the tilemap
                let s = engine.surface().window().inner_size();

                let mut x = last_mouse_pos.unwrap().x as f32 / s.width as f32;
                let mut y = last_mouse_pos.unwrap().y as f32 / s.height as f32;

                x -= 0.5;
                y -= 0.5;
                x *= 2.0;
                y *= 2.0;

                let pos = transform.screen_to_world(x, y);

                if pos.x > 0.0 && pos.y > 0.0 {
                    let grid_x = pos.x.floor() as usize;
                    let grid_y = pos.y.floor() as usize;

                    if grid_x < 16 && grid_y < 16 {
                        println!("grid {}, {}", grid_x, grid_y);

                        if let Some(mut i) = world.get_entity_mut(inspecting) {
                            if let Some(mut tilemap) = i.get_mut::<tilemap::TilemapRenderer>() {
                                tilemap.toggle(grid_x, grid_y);
                            }
                        }
                    }
                }
            }
            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state: _,
                        button: MouseButton::Right,
                        ..
                    },
                ..
            } => {}

            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                window_resized = true;
            }
            _ => (),
        }
    })
}

// mod vs {
//     vulkano_shaders::shader! {
//         ty: "vertex",
//         src: "
// #version 450

// layout(location = 0) in vec2 position;
// layout(location = 1) in vec3 color;

// layout(location = 0) out vec3 fragColor;

// layout(binding = 0) uniform Transforms{
// 	mat4 world_to_screen;
// };

// layout(binding = 1 ) buffer UniformBufferObject {
// 	vec2 offset[];
// };

// void main() {
// 	fragColor = color;
//     gl_Position = vec4(position + offset[gl_InstanceIndex] + 1 , 0.0, 1.0) * world_to_screen;
// }"
//     }
// }

// mod fs {
//     vulkano_shaders::shader! {
//         ty: "fragment",
//         src: "
// #version 450

// layout(location = 0) in vec3 color;

// layout(location = 0) out vec4 f_color;

// void main() {
//     f_color = vec4(color.rgb, 1.0);
// }"
//     }
// }

mod vs_texture {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
#version 450

layout(location = 0) in vec2 position;
layout(location = 1) in vec3 color;


layout(location = 0) out vec3 fragColor;
layout(location = 1) out vec2 uv;


layout(binding = 0) uniform Transforms{
	mat4 world_to_screen;
};

layout(binding = 1 ) buffer UniformBufferObject {
	vec2 offset[];
};


void main() {
	uv = position.xy;
	fragColor = color;
    gl_Position = vec4(position + offset[gl_InstanceIndex] , 0.0, 1.0) * world_to_screen;
}"
    }
}

mod fs_texture {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
#version 450


layout(location = 0) in vec3 color;
layout(location = 1) in vec2 uv;

layout(location = 0) out vec4 f_color;


layout(binding = 3) uniform sampler2D texSampler;


void main() {
    f_color = vec4(color.rgb , 1.0) *  texture(texSampler, uv);
}"
    }
}
