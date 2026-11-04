// Read-only editor guide, not a gameplay spawn simulation. Each two vertices
// form a separate line; CPU data comes from verified authored XML and terrain.
struct Camera {
  origin: vec4<f32>,
  forward: vec4<f32>,
  right: vec4<f32>,
  up: vec4<f32>,
}
struct GuidePoint {
  location: vec4<f32>,
  tint: vec4<f32>,
}
struct Guides { data: array<GuidePoint>, }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> guides: Guides;
struct Output {
  @builtin(position) clip: vec4<f32>,
  @location(0) tint: vec4<f32>,
}
@vertex
fn vs_wildlife(@builtin(vertex_index) vertex: u32) -> Output {
  let point = guides.data[vertex];
  let delta = point.location.xyz - camera.origin.xyz;
  let depth = dot(delta, camera.forward.xyz);
  var output: Output;
  if depth <= 0.05 {
    output.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
  } else {
    output.clip = vec4<f32>(
      dot(delta, camera.right.xyz) / (depth * camera.right.w),
      dot(delta, camera.up.xyz) / (depth * camera.up.w),
      0.9999, 1.0
    );
  }
  output.tint = point.tint;
  return output;
}
@fragment
fn fs_wildlife(input: Output) -> @location(0) vec4<f32> {
  return input.tint;
}
