pub mod compute;
pub mod gl;
pub mod uniform;

use std::ops::Mul;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use vulkano::buffer::{BufferUsage, CpuAccessibleBuffer};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, PrimaryAutoCommandBuffer, SubpassContents,
};
use vulkano::descriptor_set::{DescriptorSet, PersistentDescriptorSet, WriteDescriptorSet};
use vulkano::device::{Device, DeviceCreateInfo, Queue, QueueCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{ImageUsage, SwapchainImage};
use vulkano::instance::{Instance, InstanceCreateInfo};

use vulkano::buffer::TypedBufferAccess;
use vulkano::device::physical::{PhysicalDevice, PhysicalDeviceType};
use vulkano::device::DeviceExtensions;
use vulkano::pipeline::graphics::input_assembly::InputAssemblyState;
use vulkano::pipeline::graphics::vertex_input::BuffersDefinition;
use vulkano::pipeline::graphics::viewport::{Viewport, ViewportState};
use vulkano::pipeline::{ComputePipeline, GraphicsPipeline, Pipeline, PipelineBindPoint};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass, Subpass};
use vulkano::shader::ShaderModule;
use vulkano::swapchain::{
    self, AcquireError, Surface, Swapchain, SwapchainCreateInfo, SwapchainCreationError,
};
use vulkano::sync::{self, FlushError, GpuFuture};
use vulkano_win::VkSurfaceBuild;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, Event, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

fn main() {
    println!("Hello, world!");

    let required_extensions = vulkano_win::required_extensions();

    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::none()
    };

    let instance = Instance::new(InstanceCreateInfo {
        enabled_extensions: required_extensions,
        ..Default::default()
    })
    .expect("failed to create instance");

    let event_loop = EventLoop::new(); // ignore this for now
    let surface = WindowBuilder::new()
        .build_vk_surface(&event_loop, instance.clone())
        .unwrap();

    // pick the best physical device and queue1
    let (physical_device, graphics_queue) = PhysicalDevice::enumerate(&instance)
        .filter(|&p| p.supported_extensions().is_superset_of(&device_extensions))
        .filter_map(|p| {
            p.queue_families()
                // Find the first first queue family that is suitable.
                // If none is found, `None` is returned to `filter_map`,
                // which disqualifies this physical device.
                .find(|&q| q.supports_graphics() && q.supports_surface(&surface).unwrap_or(false))
                .map(|q| (p, q))
        })
        .min_by_key(|(p, _)| match p.properties().device_type {
            PhysicalDeviceType::DiscreteGpu => 0,
            PhysicalDeviceType::IntegratedGpu => 1,
            PhysicalDeviceType::VirtualGpu => 2,
            PhysicalDeviceType::Cpu => 3,
            PhysicalDeviceType::Other => 4,
        })
        .expect("no device available");

    //In the previous section we created an instance and chose a physical device from this instance.

    //But initialization isn't finished yet. Before being able to do anything, we have to create a device.
    //A device is an object that represents an open channel of communication with a physical device, and it is
    //probably the most important object of the Vulkan API.

    for family in physical_device.queue_families() {
        println!(
            "Found a queue family with {:?} queue(s)  [C:{:?},G:{:?},T:{:?}]",
            family.queues_count(),
            family.supports_compute(),
            family.supports_graphics(),             //supports vkDraw
            family.explicitly_supports_transfers(), //all queues can do this, but one does it better if some have this set as false
        );
    }

    //Now that we have our desired physical device, the next step is to create a logical device that can support the swapchain.

    //Creating a device returns two things:
    //- the device itself,
    //- a list of queue objects that will later allow us to submit operations.

    //Once this function call succeeds we have an open channel of communication with a Vulkan device!

    let (device, mut queues) = Device::new(
        physical_device,
        DeviceCreateInfo {
            // here we pass the desired queue families that we want to use
            queue_create_infos: vec![QueueCreateInfo::family(graphics_queue)],
            enabled_extensions: physical_device
                .required_extensions()
                .union(&device_extensions), // new
            //and everything else is set to default
            ..DeviceCreateInfo::default()
        },
    )
    .expect("failed to create device");

    let caps = physical_device
        .surface_capabilities(&surface, Default::default())
        .expect("failed to get surface capabilities");

    // this size of the swapchain images
    let dimensions = surface.window().inner_size();
    let composite_alpha = caps.supported_composite_alpha.iter().next().unwrap();
    let image_format = Some(
        physical_device
            .surface_formats(&surface, Default::default())
            .unwrap()[0]
            .0,
    );
    //  caps.min_image_count - normally 1, but all of these are effectively internal, so

    let (mut swapchain, images) = Swapchain::new(
        device.clone(),
        surface.clone(),
        SwapchainCreateInfo {
            min_image_count: caps.min_image_count + 1, // How many buffers to use in the swapchain
            image_format,
            image_extent: dimensions.into(),
            image_usage: ImageUsage::color_attachment(), // What the images are going to be used for
            composite_alpha,
            ..Default::default()
        },
    )
    .unwrap();

    //Since it is possible to request multiple queues, the queues variable returned by the function is in fact an iterator.
    //In this example code this iterator contains just one element, so let's extract it:

    //Arc is Atomic RC, reference counted box
    let queue: Arc<Queue> = queues.next().unwrap();

    //When using Vulkan, you will very often need the GPU to read or write data in memory.
    //In fact there isn't much point in using the GPU otherwise,
    //as there is nothing you can do with the results of its work except write them to memory.

    //In order for the GPU to be able to access some data
    //	(either for reading, writing or both),
    //	we first need to create a buffer object and put the data in it.

    //The most simple kind of buffer that exists is the `CpuAccessibleBuffer`, which can be created like this:

    // let data: i32 = 12;
    // let buffer = CpuAccessibleBuffer::from_data(
    //     device.clone(), //acutally just cloning the arc<>
    //     BufferUsage::all(),
    //     false,
    //     data,
    // )
    // .expect("failed to create buffer");

    //The second parameter indicates which purpose we are creating the buffer for,
    //which can help the implementation perform some optimizations.
    //Trying to use a buffer in a way that wasn't indicated in its constructor will result in an error.
    //For the sake of the example, we just create a BufferUsage that allows all possible usages.

    gl::copy_between_buffers(&device, &queue);

    compute::perform_compute(&device, &queue);
    //create the render pass and buffers
    let render_pass = gl::get_render_pass(device.clone(), swapchain.clone());
    let mut framebuffers = gl::get_framebuffers(&images, render_pass.clone());

    let vertex1 = gl::Vertex {
        position: [1., 0.],
        color: [0., 0., 1.],
    };
    let vertex2 = gl::Vertex {
        position: [0., 0.],
        color: [0., 1., 0.],
    };
    let vertex3 = gl::Vertex {
        position: [0., 1.],
        color: [1., 0., 0.],
    };
    let vertex4 = gl::Vertex {
        position: [1., 1.],
        color: [1., 0., 0.],
    };

    let vertex_buffer = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::vertex_buffer(),
        false,
        vec![vertex1, vertex2, vertex3, vertex4].into_iter(),
    )
    .unwrap();

    let index_buffer = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::index_buffer(),
        false,
        vec![0u32, 1u32, 2u32, 2u32, 0u32, 3u32].into_iter(),
    )
    .unwrap();

    let vs = vs::load(device.clone()).expect("failed to create shader module");
    let fs = fs::load(device.clone()).expect("failed to create shader module");

    let mut viewport = Viewport {
        origin: [0.0, 0.0],
        dimensions: surface.window().inner_size().into(),
        depth_range: 0.0..1.0,
    };

    let mut pipeline = gl::get_pipeline(
        device.clone(),
        vs.clone(),
        fs.clone(),
        render_pass.clone(),
        viewport.clone(),
    );

    let mut tile_positions = [[1f32, 1f32], [1f32, 1f32], [1f32, 1f32]];

    let uniform_data_buffer =
        CpuAccessibleBuffer::from_iter(device.clone(), BufferUsage::all(), false, tile_positions)
            .expect("failed to create buffer");

    //we are creating the layout for set 0
    let layout = pipeline.layout().set_layouts().get(0).unwrap();

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

    let mut transform = uniform::Transformations::new(device.clone(), pipeline.clone());

    let w_s = transform.transform();

    *w_s = glm::mat4(
        200. / dimensions.width as f32,
        0.,
        0.,
        0., //
        0.,
        200. / dimensions.height as f32,
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

    transform.update_buffer();

    let square_descriptor_set = PersistentDescriptorSet::new(
        layout.clone(),
        [
            WriteDescriptorSet::buffer(1, transform.get_buffer().clone()),
            WriteDescriptorSet::buffer(0, uniform_data_buffer.clone()),
        ], // 0 is the binding in GLSL when we use this set
    )
    .unwrap();

    let mut dragging = false;

    let mut last_mouse_pos: Option<PhysicalPosition<f64>> = None;

    event_loop.run(move |event, _, control_flow| match event {
        Event::RedrawEventsCleared => {
            if window_resized || recreate_swapchain {
                recreate_swapchain = false;

                let new_dimensions = surface.window().inner_size();

                let (new_swapchain, new_images) = match swapchain.recreate(SwapchainCreateInfo {
                    image_extent: new_dimensions.into(), // here, "image_extend" will correspond to the window dimensions
                    ..swapchain.create_info()
                }) {
                    Ok(r) => r,
                    // This error tends to happen when the user is manually resizing the window.
                    // Simply restarting the loop is the easiest way to fix this issue.
                    Err(SwapchainCreationError::ImageExtentNotSupported { .. }) => return,
                    Err(e) => panic!("Failed to recreate swapchain: {:?}", e),
                };
                swapchain = new_swapchain;
                framebuffers = gl::get_framebuffers(&new_images, render_pass.clone());

                if window_resized {
                    window_resized = false;

                    viewport.dimensions = new_dimensions.into();
                    pipeline = gl::get_pipeline(
                        device.clone(),
                        vs.clone(),
                        fs.clone(),
                        render_pass.clone(),
                        viewport.clone(),
                    );
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
            //To actually start drawing, the first thing that we need to do is to acquire an image to draw:
            let (image_i, suboptimal, acquire_future) =
                match swapchain::acquire_next_image(swapchain.clone(), None) {
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
            let framebuffer = &framebuffers[image_i];

            let cmd_buffer = {
                //build the command buffer
                let mut builder = AutoCommandBufferBuilder::primary(
                    device.clone(),
                    queue.family(),
                    CommandBufferUsage::MultipleSubmit, // don't forget to write the correct buffer usage
                )
                .unwrap();

                let render_pass = builder
                    .begin_render_pass(
                        framebuffer.clone(),
                        SubpassContents::Inline,
                        vec![[0.0, 0.0, 0.0, 1.0].into()],
                    )
                    .unwrap();

                //render pass started, can now issue draw instructions
                render_pass
                    .bind_pipeline_graphics(pipeline.clone())
                    .bind_index_buffer(index_buffer.clone())
                    .bind_vertex_buffers(0, vertex_buffer.clone())
                    .bind_descriptor_sets(
                        PipelineBindPoint::Graphics,
                        pipeline.layout().clone(),
                        0,
                        square_descriptor_set.clone(),
                    )
                    .draw_indexed(index_buffer.len() as u32, 3, 0, 0, 0)
                    .unwrap()
                    .end_render_pass()
                    .unwrap();

                //return the created command buffer
                builder.build().unwrap()
            };

            let mut i = 0f32;
            for p in &mut tile_positions[1..] {
                *p = [(t + i).cos(), (t + i).sin()];
                i += 1.;
            }

            {
                //update buffer data
                let mut w = uniform_data_buffer.write().expect("failed to write buffer");

                for (i, p) in tile_positions.iter().enumerate() {
                    w[i] = *p;
                }
            }

            //create the future to execute our command buffer
            let cmd_future = sync::now(device.clone())
                .join(acquire_future)
                .then_execute(queue.clone(), cmd_buffer)
                .unwrap();

            let execution = cmd_future
                .then_swapchain_present(queue.clone(), swapchain.clone(), image_i)
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
        } if dragging => {
            if let Some(last_pos) = last_mouse_pos {
                let diff_x = ((position.x - last_pos.x) as f32) * 2. / dimensions.width as f32;
                let diff_y = ((position.y - last_pos.y) as f32) * 2. / dimensions.height as f32;

                transform.transform().c0.w += diff_x;
                transform.transform().c1.w += diff_y;

                transform.update_buffer();
            }

            last_mouse_pos = Some(position);
        }

        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                },
            ..
        } => {
            dragging = state == ElementState::Pressed;

            if !dragging {
                last_mouse_pos = None;
            }
        }

        Event::WindowEvent {
            event: WindowEvent::Resized(_),
            ..
        } => {
            window_resized = true;
        }
        Event::MainEventsCleared => {}
        _ => (),
    });
}

mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
#version 450

layout(location = 0) in vec2 position;
layout(location = 1) in vec3 color;


layout(location = 0) out vec3 fragColor;

layout(binding = 0,set=0) buffer UniformBufferObject {
	vec2 offset[];
};

layout(binding = 1) uniform Transforms{
	mat4 world_to_screen;
};

void main() {
	fragColor = color;
    gl_Position = vec4(position + offset[gl_InstanceIndex] , 0.0, 1.0) * world_to_screen;
}"
    }
}

mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
#version 450


layout(location = 0) in vec3 color;

layout(location = 0) out vec4 f_color;

void main() {
    f_color = vec4(color.rgb, 1.0);
}"
    }
}