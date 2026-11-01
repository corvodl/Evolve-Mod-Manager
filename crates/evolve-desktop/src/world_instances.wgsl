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
struct SurfaceParams {
  cutoff: vec4<f32>,
  clouds: vec4<f32>, // authored tint, then cloud exposure or ambient share
  uv: vec4<f32>,     // authored diffuse tiling, then scroll per second
}
@group(0) @binding(4) var<uniform> authored: SurfaceParams;
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
  radial: vec4<f32>, // authored sun-side color, radial size
  top: vec4<f32>,    // authored top color, radial lobe
  ramp: vec4<f32>,   // ramp start, end, influence, global density
}
@group(0) @binding(10) var<uniform> fog: FogPreview;
// Authored TimeOfDay height/density samples. The additional binding is only
// used by the Level Editor; no original game shader code is executed.
struct HeightFogPreview {
  heights_density: vec4<f32>, // bottom height, top height, bottom density, top density
  control: vec4<f32>,         // enabled, reserved
}
@group(0) @binding(11) var<uniform> height_fog: HeightFogPreview;
// Authored terrain material high-pass color, assembled as a separate overlay.
// Linear UNORM, not sRGB: the original authored high-pass neutral is RGB 128.
// Non-terrain meshes bind alpha=0 and remain completely unaffected.
@group(0) @binding(12) var terrain_detail: texture_2d<f32>;

struct SourceLight {
  position_radius:vec4<f32>,
  direction_cone:vec4<f32>,
  color_strength:vec4<f32>,
}
struct SourceLights {
  count:vec4<f32>,
  data:array<SourceLight,24>,
}
@group(0) @binding(13) var<uniform> local_lights:SourceLights;
// Source TimeOfDay scene key, exposure envelope, film curve whitepoint and
// saturation mapped to a bounded linear-radiance ACES preview. This does
// NOT reproduce Evolve's eye-adaptation history or bloom framebuffer.
@group(0) @binding(14) var<uniform> hdr:vec4<f32>;
// Native-resolution authored Terrain.Layer diffuse sources. A 16-layer
// source-verification limit is explicit; no guessed material ID mapping.
@group(0) @binding(15) var source_terrain_layers: texture_2d_array<f32>;
@group(0) @binding(16) var source_terrain_ids: texture_2d<f32>;
struct SourceTerrainParams {
  info: vec4<f32>, // paint width, height, world unit, decoded layer count
  layers: array<vec4<f32>, 16>, // actual LayerId, TileU, TileV, detail strength
}
@group(0) @binding(17) var<uniform> source_terrain: SourceTerrainParams;
@group(0) @binding(18) var source_terrain_repeat: sampler;
@group(0) @binding(19) var source_terrain_normals: texture_2d_array<f32>;
// Tangent XY of the blended terrain layers, read by the normal step in shade.
var<private> terrain_normal_xy: vec2<f32>;

// Sun shadow cascades, nearest first. Depth runs from 0 at the sun outward.
struct SunShadows {
  matrices: array<mat4x4<f32>,4>,
  control: vec4<f32>, // enabled, texel size, ambient cube enabled, reserved
  // The map's global probe per direction: +X, -X, +Y, -Y, +Z, -Z.
  ambient_cube: array<vec4<f32>,6>,
  probe_tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> sun_shadows: SunShadows;
@group(1) @binding(1) var shadow_depth: texture_depth_2d_array;
@group(1) @binding(2) var shadow_compare: sampler_comparison;
@group(1) @binding(3) var probe_cube: texture_cube<f32>;
@group(1) @binding(4) var probe_sampler: sampler;

fn authored_ambient(n:vec3<f32>)->vec3<f32> {
  if (sun_shadows.control.z<0.5) { return light.ambient.rgb; }
  let weight=n*n;
  let x=select(sun_shadows.ambient_cube[1],sun_shadows.ambient_cube[0],n.x>=0.0).rgb;
  let y=select(sun_shadows.ambient_cube[3],sun_shadows.ambient_cube[2],n.y>=0.0).rgb;
  let z=select(sun_shadows.ambient_cube[5],sun_shadows.ambient_cube[4],n.z>=0.0).rgb;
  return x*weight.x+y*weight.y+z*weight.z;
}

fn sun_visibility(world:vec3<f32>, normal:vec3<f32>, facing:f32)->f32 {
  if (sun_shadows.control.x<0.5) { return 1.0; }
  let distance_scale=max(length(world-camera.origin.xyz)*0.0012,0.02);
  let offset=world+normal*distance_scale*(1.2-facing);
  for (var index:i32=0;index<4;index=index+1) {
    let projected=sun_shadows.matrices[index]*vec4<f32>(offset,1.0);
    if (any(abs(projected.xy)>vec2<f32>(0.985))) { continue; }
    let uv=vec2<f32>(projected.x*0.5+0.5,0.5-projected.y*0.5);
    var lit=0.0;
    for (var y:i32=-1;y<=1;y=y+1) {
      for (var x:i32=-1;x<=1;x=x+1) {
        lit+=textureSampleCompareLevel(shadow_depth,shadow_compare,
          uv+vec2<f32>(f32(x),f32(y))*sun_shadows.control.y,index,projected.z);
      }
    }
    return lit/9.0;
  }
  return 1.0;
}

fn authored_scene_linear(color:vec3<f32>)->vec3<f32> {
  // Preserve source TimeOfDay exposure and saturation in linear light.
  // The shared HDR compositor performs the only ACES tone mapping pass.
  let balanced=max(color*hdr.x*(1.0+0.05*hdr.y),vec3<f32>(0.0));
  let luma=dot(balanced,vec3<f32>(0.2126,0.7152,0.0722));
  return max(mix(vec3<f32>(luma),balanced,clamp(hdr.z,0.0,2.0)),vec3<f32>(0.0));
}

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
  // camera.origin.w carries seconds since the map opened.
  out.uv=uv*authored.uv.xy+authored.uv.zw*camera.origin.w;
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
  let safe_lod=max(implicit_lod, authored.cutoff.w);
  return textureSampleLevel(albedo,tex_sampler,uv,safe_lod);
}

// Keep the baked macro-color independent. Resolve categorical paint IDs
// from four source-neighbor texels and sample the ORIGINAL DDS mip chain per
// fragment; this preserves close-up source detail absent from a CPU atlas.
// The boundary weights are a documented preview approximation, not native
// CryEngine paint-mask, POM or slope/blend behavior.
fn original_terrain_detail(input:Output)->vec3<f32> {
  let extents=max(source_terrain.info.xy-vec2<f32>(1.0),vec2<f32>(1.0));
  let coord=clamp(input.uv,vec2<f32>(0.0),vec2<f32>(1.0))*extents;
  let lo=vec2<i32>(floor(coord));
  let hi=min(lo+vec2<i32>(1,1),vec2<i32>(source_terrain.info.xy)-vec2<i32>(1,1));
  let a=fract(coord);
  let ids=array<vec2<i32>,4>(
    lo,vec2<i32>(hi.x,lo.y),vec2<i32>(lo.x,hi.y),hi
  );
  let weights=array<f32,4>(
    (1.0-a.x)*(1.0-a.y),a.x*(1.0-a.y),
    (1.0-a.x)*a.y,a.x*a.y
  );
  let dx=dpdx(input.uv);
  let dy=dpdy(input.uv);
  let original_pixels=vec2<f32>(textureDimensions(source_terrain_layers,0));
  var signed_color=vec3<f32>(0.0);
  for (var neighbor:i32=0;neighbor<4;neighbor=neighbor+1) {
    if (weights[neighbor]<=0.0) {continue;}
    let encoded=textureLoad(source_terrain_ids,ids[neighbor],0).r;
    let paint_id=round(encoded*255.0);
    for (var layer:i32=0;layer<16;layer=layer+1) {
      if (f32(layer)>=source_terrain.info.w) {break;}
      let config=source_terrain.layers[layer];
      if (abs(config.x-paint_id)>0.25) {continue;}
      let tile=extents*source_terrain.info.z*config.yz;
      let uv=input.uv*tile;
      let diff_x=dx*tile*original_pixels;
      let diff_y=dy*tile*original_pixels;
      let footprint=max(max(length(diff_x),length(diff_y)),1.0);
      let lod=log2(footprint);
      let detail=textureSampleLevel(
        source_terrain_layers,source_terrain_repeat,uv,layer,lod
      ).rgb;
      signed_color+=(detail-vec3<f32>(128.0/255.0))*config.w*weights[neighbor];
      let layer_normal=textureSampleLevel(
        source_terrain_normals,source_terrain_repeat,uv,layer,lod
      ).rg*2.0-vec2<f32>(1.0);
      terrain_normal_xy+=layer_normal*weights[neighbor];
      break;
    }
  }
  return signed_color;
}

// Shared lighting, preserving the original source alpha only for materials
// explicitly routed through the transparent pipeline. CryEngine can store
// gloss, masks and other non-opacity data in a diffuse texture's alpha.
fn shade(input:Output, use_opacity_mask:bool)->vec4<f32> {
  var base=sample_source_albedo(input.uv);
  // CryEngine Terrain.Layer high-pass is ADD-SIGNED over the original
  // low-frequency baked terrain color, not a diffuse replacement.
  // Prefer original DDS layers at their full mip dimensions when same-build
  // sources form a supported texture array; otherwise use a mip-filtered
  // CPU fallback without a guessed fixed-distance fade.
  var signed_detail=vec3<f32>(0.0);
  if (source_terrain.info.w>0.5) {
    signed_detail=original_terrain_detail(input);
  } else {
    let detail=textureSample(terrain_detail,tex_sampler,input.uv);
    signed_detail=(detail.rgb-vec3<f32>(128.0/255.0))*detail.a;
  }
  base=vec4<f32>(clamp(base.rgb+signed_detail,vec3<f32>(0.0),vec3<f32>(1.0))*authored.clouds.rgb,base.a);
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
  if (dot(terrain_normal_xy,terrain_normal_xy)>0.000001) {
    // Painted layers tile across world XY, so that plane is their tangent basis.
    let t=normalize(vec3<f32>(1.0,0.0,0.0)-n*n.x);
    let b=normalize(vec3<f32>(0.0,1.0,0.0)-n*n.y);
    let z=sqrt(max(0.0,1.0-min(dot(terrain_normal_xy,terrain_normal_xy),1.0)));
    n=normalize(t*terrain_normal_xy.x+b*terrain_normal_xy.y+n*z);
  }

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
  if (authored.cutoff.z>0.5 && abs(determinant)>0.000001) {
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
  let visibility=sun_visibility(input.world,normalize(input.normal+vec3<f32>(0.0,0.0,0.0001)),lambert);
  let sun=light.sun_color.rgb*(light.sun.w*lambert*visibility);
  // Source-authored specular reflectance and gloss are separate channels.
  // A missing specular map is black (never invent shiny materials).
  let reflectance=textureSample(specular_map,tex_sampler,input.uv).rgb;
  let gloss=textureSample(gloss_map,tex_sampler,input.uv).r;
  let view_direction=normalize(camera.origin.xyz-input.world);
  let half_direction=normalize(sun_direction+view_direction);
  let exponent=mix(8.0,120.0,clamp(gloss,0.0,1.0));
  let highlight=pow(max(dot(n,half_direction),0.0),exponent);
  let specular=reflectance*light.sun_color.rgb*(light.sun.w*0.6*highlight*visibility);
  // Reflect the map's baked probe. Only materials with authored specular do.
  let view_angle=1.0-max(dot(n,view_direction),0.0);
  let fresnel=reflectance+(vec3<f32>(1.0)-reflectance)*pow(view_angle,5.0);
  let has_specular=step(0.001,max(reflectance.r,max(reflectance.g,reflectance.b)));
  let reflected=textureSampleLevel(probe_cube,probe_sampler,
    reflect(-view_direction,n),0.0).rgb*sun_shadows.probe_tint.rgb*fresnel*has_specular;
  var local_radiance=vec3<f32>(0.0);
  let sources=u32(clamp(local_lights.count.x,0.0,24.0));
  for (var i=0u;i<24u;i=i+1u) {
    if (i>=sources) { break; }
    let source=local_lights.data[i];
    let delta=source.position_radius.xyz-input.world;
    let radius=max(source.position_radius.w,0.001);
    let dist=length(delta);
    if (dist>=radius || dist<0.00001) {continue;}
    let incoming=delta/dist;
    var attenuation=pow(1.0-clamp(dist/radius,0.0,1.0),2.0);
    // A projector only affects the authored forward cone, with a bounded
    // penumbra preview (cookie and dynamic shadows require engine support).
    if (source.direction_cone.w>=0.0) {
      let projector=dot(-incoming,normalize(source.direction_cone.xyz));
      attenuation*=smoothstep(
         source.direction_cone.w,
         min(source.direction_cone.w+0.08,1.0),
         projector);
    }
    let diffuse=max(dot(n,incoming),0.0);
    let local_half=normalize(incoming+view_direction);
    let spec=reflectance*
       pow(max(dot(n,local_half),0.0),exponent)*0.15;
    local_radiance+=(base.rgb*diffuse+spec)*
      source.color_strength.rgb*
      source.color_strength.w*attenuation;
  }
  // A bounded viewport preview only: original GlowAmount is not modified,
  // and this is not a claim of matching native bloom / HDR.
  let emission=textureSample(emissive_map,tex_sampler,input.uv).rgb;
  // Source GlowAmount is the strength of verified emissive/Emittance
  // channels. Do not divide it by three and normalize it away before HDR;
  // the filmic output pass bounds highlights, while true bloom remains a
  // separate post-processing feature. No guessed emission for plain Illum.
  let preview_glow=emission*max(authored.cutoff.y,0.0)*0.65;
  return vec4<f32>(base.rgb*(authored_ambient(n)*authored.clouds.w+sun)+specular+reflected+local_radiance+preview_glow,base.a);
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
  let to_fragment=input.world-camera.origin.xyz;
  let distance=length(to_fragment);
  var depth=distance*fog.params.y*fog.params.z;
  if (height_fog.control.x > 0.5) {
    depth=height_fog_depth(input,distance);
    let span=max(height_fog.heights_density.y-height_fog.heights_density.x,0.01);
    let h=clamp((input.world.z-height_fog.heights_density.x)/span,0.0,1.0);
    let graded=h*(2.0-h);
    let ramp_span=max(fog.ramp.y-fog.ramp.x,0.01);
    var ramp=clamp((distance-fog.ramp.x)/ramp_span,0.0,1.0);
    ramp=mix(1.0,ramp*(2.0-ramp),fog.ramp.z);
    var haze=mix(fog.color.rgb,fog.top.rgb,graded);
    // Authored sun-side lobe. Its shape is calibrated, not decoded engine math.
    let reach=clamp(distance*fog.params.y,0.0,1.0);
    let sharpness=1.0/max(1.0-fog.radial.w,0.001);
    let toward_sun=dot(to_fragment/max(distance,0.001),normalize(light.sun.xyz));
    let lobe=pow(reach,fog.top.w)*pow(2.0,(toward_sun-1.0)*sharpness);
    haze+=fog.radial.rgb*lobe;
    let amount=(1.0-exp(-depth*fog.ramp.w))*ramp;
    return vec4<f32>(mix(color.rgb,haze,clamp(amount,0.0,1.0)),color.a);
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
  return vec4<f32>(authored_scene_linear(fogged.rgb),1.0);
}

@fragment
fn fs_foliage(input:Output)->@location(0) vec4<f32> {
  // CryEngine Vegetation: cut the original diffuse alpha at 50% (default
  // AlphaTest). The separate Opacity map is leaf thickness/transmittance.
  // Discarded fragments neither write depth nor reveal the desktop; surviving
  // fragments write opaque window alpha and regular world depth.
  let leaf_alpha=sample_source_albedo(input.uv).a;
  if (leaf_alpha < authored.cutoff.x) { discard; }
  let color=shade(input,false);
  let fogged=distance_fog(input,color);
  return vec4<f32>(authored_scene_linear(fogged.rgb),1.0);
}

// Painted cloud domes are unlit and blend over the sky behind all geometry.
@fragment
fn fs_distance_clouds(input:Output)->@location(0) vec4<f32> {
  let source=sample_source_albedo(input.uv);
  return vec4<f32>(source.rgb*authored.clouds.rgb*authored.clouds.w,source.a);
}

@fragment
fn fs_main(input:Output)->@location(0) vec4<f32> {
  // Glass/water opacity is continuous coverage, not an alpha-cutout test.
  // A 20% source opacity must stay visible as 20% blended glass instead of
  // being discarded by the solid-surface 50% mask threshold.
  let color=shade(input,false);
  let fogged=distance_fog(input,color);
  let alpha=textureSample(authored_opacity,tex_sampler,input.uv).r;
  return vec4<f32>(authored_scene_linear(fogged.rgb),color.a*alpha);
}