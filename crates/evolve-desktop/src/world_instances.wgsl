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
// The fourth alpha uniform component is the minimum fully uploaded mip
// level for a progressive diffuse; fragment sampling must not access a
// not-yet-populated higher-resolution GPU mip.
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
// A non-native distance-only preview from the exact map's Environment.xml and
// original TimeOfDay.xml Fog color. The renderer zeros params when either is
// unavailable. This deliberately omits CryEngine height/volumetric fog.
struct FogPreview {
  color: vec4<f32>,
  params: vec4<f32>, // enabled, inverse ViewDistance, scaled LDR density, reserved
}
@group(0) @binding(10) var<uniform> fog: FogPreview;
// Authored TimeOfDay height/density samples. The additional binding is only
// used by the Level Editor; no original game shader code is executed.
struct HeightFogPreview {
  heights_density: vec4<f32>, // bottom height, top height, bottom density, top density
  control: vec4<f32>,         // enabled, reserved
}
@group(0) @binding(11) var<uniform> height_fog: HeightFogPreview;
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
// Match ordinary implicit mip LOD while clamping to the source levels that
// have actually arrived on the GPU. Once streaming reaches level zero this
// reproduces normal trilinear level selection, not a permanently blurry mip.
fn sample_source_albedo(uv:vec2<f32>)->vec4<f32> {
  let size=vec2<f32>(textureDimensions(albedo,0));
  let dx=dpdx(uv)*size;
  let dy=dpdy(uv)*size;
  let area=max(max(dot(dx,dx),dot(dy,dy)),0.000001);
  let implicit_lod=max(0.0,0.5*log2(area));
  let safe_lod=max(implicit_lod, authored_cutoff.w);
  return textureSampleLevel(albedo,tex_sampler,uv,safe_lod);
}

// Shared lighting, preserving the original source alpha only for materials
// explicitly routed through the transparent pipeline. CryEngine can store
// gloss, masks and other non-opacity data in a diffuse texture's alpha.
fn shade(input:Output, use_opacity_mask:bool)->vec4<f32> {
  let base=sample_source_albedo(input.uv);
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
      let orthogonal_bitangent=bitangent-t*dot(t,bitangent);
      if (dot(orthogonal_bitangent,orthogonal_bitangent)>0.000001) {
        let b=normalize(orthogonal_bitangent);
        n=normalize(t*xy.x+b*xy.y+n*tangent_z);
      }
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

// A visually bounded preview heuristic, not an Evolve fog equation. Applying
// this in the WORLD shader only leaves the separate Model Viewer unchanged.
// Analytic integral of a vertically exponential density profile along the
// camera-to-fragment ray. Horizontal rays have a stable Taylor-series path.
// Zero-density endpoints are rejected by the source sampler; no fabricated
// epsilon endpoint is uploaded to this analytic exponential profile.
// Density scaling still uses PR #157's *preview* distance calibration: this
// is NOT an implementation of the Evolve engine's source shader equation.
fn height_fog_depth(input:Output, distance:f32)->f32 {
  let h=height_fog.heights_density;
  let bottom_density=max(h.z,0.000001);
  let top_density=max(h.w,0.000001);
  let falloff=clamp(log(top_density/bottom_density)/max(h.y-h.x,0.01),-0.01,0.01);
  let at_camera=bottom_density*exp(clamp(falloff*(camera.origin.z-h.x),-12.0,12.0));
  let delta=falloff*(input.world.z-camera.origin.z);
  var integral_ratio=1.0+0.5*delta+delta*delta/6.0;
  if (abs(delta)>=0.001) {
    integral_ratio=(exp(clamp(delta,-12.0,12.0))-1.0)/delta;
  }
  return clamp(at_camera*distance*max(integral_ratio,0.0)*fog.params.y*fog.params.z,0.0,12.0);
}

fn distance_fog(input:Output, color:vec4<f32>)->vec4<f32> {
  if (fog.params.x < 0.5) { return color; }
  let distance=length(camera.origin.xyz-input.world);
  var depth=distance*fog.params.y*fog.params.z;
  if (height_fog.control.x > 0.5) {
    depth=height_fog_depth(input,distance);
  }
  let amount=clamp(1.0-exp(-depth),0.0,0.92);
  return vec4<f32>(mix(color.rgb,fog.color.rgb,amount),color.a);
}

// Diagnostic source-color mode ONLY for the original ground atlas. It
// bypasses the approximate preview sun, material BRDF and fog but keeps
// the GPU sRGB decode/output path intact. No source pixels are changed.
@fragment
fn fs_terrain_source(input:Output)->@location(0) vec4<f32> {
  let raw=sample_source_albedo(input.uv);
  return vec4<f32>(raw.rgb,1.0);
}

@fragment
fn fs_opaque(input:Output)->@location(0) vec4<f32> {
  // World geometry renders directly into the native Iced window surface.
  // Without this alpha override, a cliff/terrain/prop using packed diffuse
  // alpha can punch a transparent hole in the editor window and reveal the
  // desktop or Activity window behind it, even over the opaque backdrop.
  let color=shade(input,true);
  let fogged=distance_fog(input,color);
  return vec4<f32>(fogged.rgb,1.0);
}

@fragment
fn fs_foliage(input:Output)->@location(0) vec4<f32> {
  // CryEngine Vegetation: cut the original diffuse alpha at 50% (default
  // AlphaTest). The separate Opacity map is leaf thickness/transmittance.
  // Discarded fragments neither write depth nor reveal the desktop; surviving
  // fragments write opaque window alpha and regular world depth.
  let leaf_alpha=sample_source_albedo(input.uv).a;
  if (leaf_alpha < authored_cutoff.x) { discard; }
  let color=shade(input,false);
  let fogged=distance_fog(input,color);
  return vec4<f32>(fogged.rgb,1.0);
}

@fragment
fn fs_main(input:Output)->@location(0) vec4<f32> {
  // Glass/water opacity is continuous coverage, not an alpha-cutout test.
  // A 20% source opacity must stay visible as 20% blended glass instead of
  // being discarded by the solid-surface 50% mask threshold.
  let color=shade(input,false);
  let fogged=distance_fog(input,color);
  let alpha=textureSample(authored_opacity,tex_sampler,input.uv).r;
  return vec4<f32>(fogged.rgb,color.a*alpha);
}