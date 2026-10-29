// World-space instanced CryEngine geometry, one static mesh per GPU group.
// All movement/projection is GPU-side; no per-placement CPU vertex expansion.
struct Camera {
  origin: vec4<f32>,
  forward: vec4<f32>,
  right: vec4<f32>,
  up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var albedo: texture_2d<f32>;
@group(0) @binding(3) var authored_opacity: texture_2d<f32>;
struct Output {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) world: vec3<f32>,
}
@vertex
fn vs_main(
  @location(0) xyz: vec3<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) normal: vec3<f32>,
  @location(3) col0: vec4<f32>,
  @location(4) col1: vec4<f32>,
  @location(5) col2: vec4<f32>,
  @location(6) col3: vec4<f32>,
) -> Output {
  let model=mat4x4<f32>(col0,col1,col2,col3);
  let world=(model*vec4<f32>(xyz,1.0)).xyz;
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.clip=vec4<f32>(
      dot(delta,camera.right.xyz)/camera.right.w,
      dot(delta,camera.up.xyz)/camera.up.w,
      d-camera.forward.w,d);
  out.uv=uv;
  out.normal=normalize(mat3x3<f32>(col0.xyz,col1.xyz,col2.xyz)*normal);
  out.world=world;
  return out;
}
// Shared lighting, preserving the original source alpha only for materials
// explicitly routed through the transparent pipeline. CryEngine can store
// gloss, masks and other non-opacity data in a diffuse texture's alpha.
fn shade(input:Output)->vec4<f32> {
  let base=textureSample(albedo,tex_sampler,input.uv);
  let coverage=textureSample(authored_opacity,tex_sampler,input.uv).r;
  if (coverage < 0.5) { discard; }
  var n=input.normal;
  if length(n)<0.01 {
    n=normalize(cross(dpdx(input.world),dpdy(input.world)));
  }
  let sunlight=abs(dot(normalize(n),normalize(vec3<f32>(0.48,-0.46,0.75))));
  return vec4<f32>(base.rgb*(0.35+0.65*sunlight),base.a);
}

@fragment
fn fs_opaque(input:Output)->@location(0) vec4<f32> {
  // World geometry renders directly into the native Iced window surface.
  // Without this alpha override, a cliff/terrain/prop using packed diffuse
  // alpha can punch a transparent hole in the editor window and reveal the
  // desktop or Activity window behind it, even over the opaque backdrop.
  let color=shade(input);
  return vec4<f32>(color.rgb,1.0);
}

@fragment
fn fs_main(input:Output)->@location(0) vec4<f32> {
  // Preserve real authored transparency in the separately blended pipeline.
  return shade(input);
}