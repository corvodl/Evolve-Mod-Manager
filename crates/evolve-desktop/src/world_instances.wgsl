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
// Material-authored AlphaTest normalized to 0..1. The default 0.5 is
// supplied only if the selected original MTL did not specify a threshold.
@group(0) @binding(4) var<uniform> authored_cutoff: vec4<f32>;
// Viewport-only time-of-day lighting. Never reads or edits game engine state.
struct TimeOfDayLight {
  sun: vec4<f32>,
  ambient: vec4<f32>,
  sun_color: vec4<f32>,
}
@group(0) @binding(5) var<uniform> light: TimeOfDayLight;
// Source MTL maps are optional and always have neutral 1x1 fallbacks.
@group(0) @binding(6) var normal_map: texture_2d<f32>;
@group(0) @binding(7) var specular_map: texture_2d<f32>;
@group(0) @binding(8) var gloss_map: texture_2d<f32>;
@group(0) @binding(9) var emissive_map: texture_2d<f32>;
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
  // Source actors may scale their X/Y/Z axes independently. Transforming
  // normals with the model matrix makes cliffs, buildings and foliage shade
  // incorrectly. The cofactor columns are the inverse-transpose basis;
  // divide by determinant to preserve reflections / negative axis scale.
  let a=col0.xyz;
  let b=col1.xyz;
  let c=col2.xyz;
  let det=dot(a,cross(b,c));
  var transformed_normal=vec3<f32>(0.0);
  if (abs(det)>0.00000001 && dot(normal,normal)>0.00000001) {
    transformed_normal=(cross(b,c)*normal.x+cross(c,a)*normal.y+cross(a,b)*normal.z)/det;
  }
  out.normal=transformed_normal;
  out.world=world;
  return out;
}
// Shared lighting, preserving the original source alpha only for materials
// explicitly routed through the transparent pipeline. CryEngine can store
// gloss, masks and other non-opacity data in a diffuse texture's alpha.
fn shade(input:Output, use_opacity_mask:bool)->vec4<f32> {
  let base=textureSample(albedo,tex_sampler,input.uv);
  if (use_opacity_mask) {
    let coverage=textureSample(authored_opacity,tex_sampler,input.uv).r;
    if (coverage < 0.5) { discard; }
  }
  // When original mesh normals are absent, use the actual world-space face
  // derivatives rather than inventing a constant upward vector.
  let face_normal = cross(dpdx(input.world), dpdy(input.world));
  var n=input.normal;
  if (dot(n,n)<0.000001) { n=face_normal; }
  if (dot(n,n)<0.000001) { n=vec3<f32>(0.0,0.0,1.0); }
  n=normalize(n);

  // Original BC5 normal maps store tangent XY in linear space. Reconstruct Z,
  // then derive a basis from the unchanged original UV0 and world geometry.
  // This handles winding, UV mirrors and non-uniform instance transforms.
  let uv_dx=dpdx(input.uv);
  let uv_dy=dpdy(input.uv);
  let pos_dx=dpdx(input.world);
  let pos_dy=dpdy(input.world);
  let determinant=uv_dx.x*uv_dy.y-uv_dx.y*uv_dy.x;
  let encoded=textureSample(normal_map,tex_sampler,input.uv).rg;
  let xy=encoded*2.0-vec2<f32>(1.0);
  let tangent_z=sqrt(max(0.0,1.0-dot(xy,xy)));
  if (authored_cutoff.z>0.5 && abs(determinant)>0.000001) {
    let tangent_raw=(pos_dx*uv_dy.y-pos_dy*uv_dx.y)/determinant;
    let bitangent_raw=(pos_dy*uv_dx.x-pos_dx*uv_dy.x)/determinant;
    let tangent=tangent_raw-n*dot(n,tangent_raw);
    let bitangent=bitangent_raw-n*dot(n,bitangent_raw);
    if (dot(tangent,tangent)>0.000001 && dot(bitangent,bitangent)>0.000001) {
      let t=normalize(tangent);
      let b=normalize(bitangent-t*dot(t,bitangent));
      n=normalize(t*xy.x+b*xy.y+n*tangent_z);
    }
  }

  let sun_direction=normalize(light.sun.xyz);
  let lambert=max(dot(n,sun_direction),0.0);
  let sun=light.sun_color.rgb*(light.sun.w*lambert);
  // Source-authored specular reflectance and gloss are separate channels.
  // A missing specular map is black (never invent shiny materials).
  let reflectance=textureSample(specular_map,tex_sampler,input.uv).rgb;
  let gloss=textureSample(gloss_map,tex_sampler,input.uv).r;
  let view_direction=normalize(camera.origin.xyz-input.world);
  let half_direction=normalize(sun_direction+view_direction);
  let exponent=mix(8.0,120.0,clamp(gloss,0.0,1.0));
  let highlight=pow(max(dot(n,half_direction),0.0),exponent);
  let specular=reflectance*light.sun_color.rgb*(light.sun.w*0.6*highlight);
  // A bounded viewport preview only: original GlowAmount is not modified,
  // and this is not a claim of matching native bloom / HDR.
  let emission=textureSample(emissive_map,tex_sampler,input.uv).rgb;
  let glow=emission*(authored_cutoff.y/3.0)*0.45;
  let glow_peak=max(max(glow.r,glow.g),glow.b);
  let preview_glow=glow/(1.0+glow_peak);
  return vec4<f32>(base.rgb*(light.ambient.rgb+sun)+specular+preview_glow,base.a);
}

@fragment
fn fs_opaque(input:Output)->@location(0) vec4<f32> {
  // World geometry renders directly into the native Iced window surface.
  // Without this alpha override, a cliff/terrain/prop using packed diffuse
  // alpha can punch a transparent hole in the editor window and reveal the
  // desktop or Activity window behind it, even over the opaque backdrop.
  let color=shade(input,true);
  return vec4<f32>(color.rgb,1.0);
}

@fragment
fn fs_foliage(input:Output)->@location(0) vec4<f32> {
  // CryEngine Vegetation: cut the original diffuse alpha at 50% (default
  // AlphaTest). The separate Opacity map is leaf thickness/transmittance.
  // Discarded fragments neither write depth nor reveal the desktop; surviving
  // fragments write opaque window alpha and regular world depth.
  let leaf_alpha=textureSample(albedo,tex_sampler,input.uv).a;
  if (leaf_alpha < authored_cutoff.x) { discard; }
  let color=shade(input,false);
  return vec4<f32>(color.rgb,1.0);
}

@fragment
fn fs_main(input:Output)->@location(0) vec4<f32> {
  // Preserve real authored transparency in the separately blended pipeline.
  return shade(input,true);
}