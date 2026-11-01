// Depth-only sun shadow cascades for the instanced Level Editor world.
// Vegetation keeps its authored alpha test so leaves cast leaf-shaped shadows.
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var albedo: texture_2d<f32>;
struct SurfaceParams {
  cutoff: vec4<f32>,
  clouds: vec4<f32>,
  uv: vec4<f32>,
}
@group(0) @binding(4) var<uniform> authored: SurfaceParams;
@group(1) @binding(0) var<uniform> cascade: mat4x4<f32>;

struct Output {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
}

@vertex
fn vs_shadow(
  @location(0) xyz: vec3<f32>,
  @location(1) uv: vec2<f32>,
  @location(3) col0: vec4<f32>,
  @location(4) col1: vec4<f32>,
  @location(5) col2: vec4<f32>,
  @location(6) col3: vec4<f32>,
) -> Output {
  let model=mat4x4<f32>(col0,col1,col2,col3);
  var out:Output;
  out.clip=cascade*(model*vec4<f32>(xyz,1.0));
  out.uv=uv*authored.uv.xy;
  return out;
}

@fragment
fn fs_shadow_cutout(input:Output) {
  // Only mips that have already streamed in may be read.
  let alpha=textureSampleLevel(albedo,tex_sampler,input.uv,authored.cutoff.w).a;
  if (alpha<authored.cutoff.x) { discard; }
}
