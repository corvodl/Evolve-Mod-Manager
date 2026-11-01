// World-space instanced CryEngine geometry, one static mesh per GPU group.
// All movement/projection is GPU-side; no per-placement CPU vertex expansion.
struct Camera {
  origin: vec4<f32>,
  forward: vec4<f32>,
  right: vec4<f32>,
  up: vec4<f32>,
  reflection_height: vec4<f32>,
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
  specular: vec4<f32>, // authored Specular color
  waterfall_scroll: vec4<f32>, // TillingLayer0, TillingLayer1, SpeedLayer0, SpeedLayer1
  waterfall_surface: vec4<f32>, // BumpScaleLayer0, BumpScaleLayer1, ReflectionAmount, FresnelBias
  waterfall_blend: vec4<f32>, // AlphaBlendMul, FoamDeform, FoamMultiplier
  detail: vec4<f32>,  // Detail tiling U and V, DetailBumpScale, DetailDiffuseScale
  layers: vec4<f32>,  // DetailGlossScale, has detail, has second layer, BlendFactor
  blend: vec4<f32>,   // BlendFalloff, BlendLayer2Tiling, BlendMaskTiling
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
  bulb:vec4<f32>,
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
  blend: array<vec4<f32>, 16>,  // BlendFactor, BlendFalloff, PomDisplacement
}
@group(0) @binding(17) var<uniform> source_terrain: SourceTerrainParams;
@group(0) @binding(18) var source_terrain_repeat: sampler;
@group(0) @binding(19) var source_terrain_normals: texture_2d_array<f32>;
@group(0) @binding(20) var source_terrain_heights: texture_2d_array<f32>;
// BLENDLAYER's second diffuse and mask. Binding 12 is the Detail map on non-terrain.
@group(0) @binding(21) var blend_layer: texture_2d<f32>;
@group(0) @binding(22) var blend_mask: texture_2d<f32>;
// Tangent XY of the blended terrain layers, read by the normal step in shade.
var<private> terrain_normal_xy: vec2<f32>;
var<private> blended_surface: f32;

// Sun shadow cascades, nearest first. Depth runs from 0 at the sun outward.
struct SunShadows {
  matrices: array<mat4x4<f32>,4>,
  control: vec4<f32>, // enabled, texel size, ambient cube enabled, wet surface darkening
  // The map's global probe per direction: +X, -X, +Y, -Y, +Z, -Z.
  ambient_cube: array<vec4<f32>,6>,
  probe_tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> sun_shadows: SunShadows;
@group(1) @binding(1) var shadow_depth: texture_depth_2d_array;
@group(1) @binding(2) var shadow_compare: sampler_comparison;
@group(1) @binding(3) var probe_cube: texture_cube<f32>;
@group(1) @binding(4) var probe_sampler: sampler;
struct LocalProbe {
  center: vec4<f32>,          // world position, cube array index
  axes: array<vec4<f32>,3>,   // world axis, inverse half size
  tint: vec4<f32>,            // authored color, falloff
  ambient: array<vec4<f32>,6>,
}
struct LocalProbes {
  count: vec4<f32>,
  data: array<LocalProbe,12>,
}
@group(1) @binding(5) var<uniform> local_probes: LocalProbes;
@group(1) @binding(6) var local_probe_cubes: texture_cube_array<f32>;

// The game's box falloff: map into the box, round its corners, then smoothstep.
fn local_probe_weight(probe:LocalProbe,world:vec3<f32>)->f32 {
  let offset=probe.center.xyz-world;
  let p=clamp(vec3<f32>(dot(probe.axes[0].xyz,offset)*probe.axes[0].w,
    dot(probe.axes[1].xyz,offset)*probe.axes[1].w,
    dot(probe.axes[2].xyz,offset)*probe.axes[2].w),vec3<f32>(-1.0),vec3<f32>(1.0));
  let s=p.zxy*p.zxy;
  let squeezed=p*sqrt(vec3<f32>(1.0)-s*0.5-s.zxy*0.5+s.zxy*s*0.333);
  let w=clamp(clamp(1.0-dot(squeezed,squeezed),0.0,1.0)/probe.tint.w,0.0,1.0);
  return w*w*(3.0-2.0*w);
}

fn face_ambient(faces:array<vec4<f32>,6>,n:vec3<f32>)->vec3<f32> {
  let weight=n*n;
  let x=select(faces[1],faces[0],n.x>=0.0).rgb;
  let y=select(faces[3],faces[2],n.y>=0.0).rgb;
  let z=select(faces[5],faces[4],n.z>=0.0).rgb;
  return x*weight.x+y*weight.y+z*weight.z;
}

// A probe's authored color without its diffuse multiplier. Applying the multiplier
// to reflections leaves hot patches on rock, so this is calibrated.
fn tint_color(tint:vec3<f32>)->vec3<f32> {
  return tint/max(max(tint.r,max(tint.g,tint.b)),0.001);
}

struct ProbeLight { ambient:vec3<f32>, environment:vec3<f32>, }

// The global probe, with each nearby box blended over it in priority order.
fn probe_light(world:vec3<f32>,n:vec3<f32>,reflected:vec3<f32>,lod:f32)->ProbeLight {
  var out:ProbeLight;
  out.ambient=authored_ambient(n);
  out.environment=textureSampleLevel(probe_cube,probe_sampler,reflected,lod).rgb*tint_color(sun_shadows.probe_tint.rgb);
  let probe_count=u32(clamp(local_probes.count.x,0.0,12.0));
  for (var i=0u;i<probe_count;i=i+1u) {
    let probe=local_probes.data[i];
    let weight=local_probe_weight(probe,world);
    if (weight<=0.001) {continue;}
    out.ambient=mix(out.ambient,face_ambient(probe.ambient,n),weight);
    out.environment=mix(out.environment,textureSampleLevel(local_probe_cubes,probe_sampler,
      reflected,i32(probe.center.w),max(lod-2.0,0.0)).rgb*tint_color(probe.tint.rgb),weight);
  }
  return out;
}

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
  let balanced=max(color,vec3<f32>(0.0));
  let luma=dot(balanced,vec3<f32>(0.2126,0.7152,0.0722));
  return max(mix(vec3<f32>(luma),balanced,clamp(hdr.z,0.0,2.0)),vec3<f32>(0.0));
}

struct Output {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) world: vec3<f32>,
  @location(3) paint: f32,
}
@vertex
fn vs_main(
  @location(0) xyz: vec3<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) normal: vec3<f32>,
  @location(7) paint: f32,
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
  out.paint=paint;
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

// The game's Terrain.Layer blend: each painted layer adds its signed detail to
// the display-encoded baked color, weighted by height. Returns display color.
fn undo_srgb(c:vec3<f32>)->vec3<f32> {
  let high=pow(max(c,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))*1.055-vec3<f32>(0.055);
  return select(high,c*12.92,c<vec3<f32>(0.003131));
}

fn blended_terrain(input:Output,baked:vec3<f32>)->vec3<f32> {
  let extents=max(source_terrain.info.xy-vec2<f32>(1.0),vec2<f32>(1.0));
  let coord=clamp(input.uv,vec2<f32>(0.0),vec2<f32>(1.0))*extents;
  let lo=vec2<i32>(floor(coord));
  let hi=min(lo+vec2<i32>(1,1),vec2<i32>(source_terrain.info.xy)-vec2<i32>(1,1));
  let a=fract(coord);
  let cells=array<vec2<i32>,4>(lo,vec2<i32>(hi.x,lo.y),vec2<i32>(lo.x,hi.y),hi);
  let corner=array<f32,4>((1.0-a.x)*(1.0-a.y),a.x*(1.0-a.y),(1.0-a.x)*a.y,a.x*a.y);
  var coverage=array<f32,16>();
  let count=i32(source_terrain.info.w);
  for (var i:i32=0;i<4;i=i+1) {
    let id=round(textureLoad(source_terrain_ids,cells[i],0).r*255.0);
    for (var layer:i32=0;layer<count;layer=layer+1) {
      if (abs(source_terrain.layers[layer].x-id)<=0.25) {
        coverage[layer]+=corner[i];
        break;
      }
    }
  }
  let dx=dpdx(input.uv);
  let dy=dpdy(input.uv);
  let original_pixels=vec2<f32>(textureDimensions(source_terrain_layers,0));
  var color=baked;
  for (var layer:i32=0;layer<count;layer=layer+1) {
    let weight=coverage[layer];
    if (weight<=0.001) {continue;}
    let config=source_terrain.layers[layer];
    let tile=extents*source_terrain.info.z*config.yz;
    let lod=log2(max(max(length(dx*tile*original_pixels),length(dy*tile*original_pixels)),1.0));
    var uv=input.uv*tile;
    // The game's parallax: start offset toward the camera at full height and
    // march back to the surface point at height zero. Scale follows paint weight
    // and fades with distance, and never stretches with the view angle.
    let to_camera=camera.origin.xyz-input.world;
    let reach=length(to_camera);
    let fade=clamp((40.0-reach)/15.0,0.0,1.0);
    let pom=source_terrain.blend[layer].z*weight*fade;
    if (pom>0.0001) {
      let offset=to_camera.xy/max(reach,0.001)*pom;
      var at=uv+offset;
      var level=1.0;
      var sampled=textureSampleLevel(source_terrain_heights,source_terrain_repeat,at,layer,lod).r;
      var previous=at;
      var previous_gap=level-sampled;
      for (var i=0;i<15 && sampled<level;i=i+1) {
        previous=at;
        previous_gap=level-sampled;
        at-=offset/15.0;
        level-=1.0/15.0;
        sampled=textureSampleLevel(source_terrain_heights,source_terrain_repeat,at,layer,lod).r;
      }
      // One refinement between the last two samples.
      let over=max(sampled-level,0.0);
      uv=mix(at,previous,clamp(over/max(over+previous_gap,0.0001),0.0,1.0));
    }
    let detail=textureSampleLevel(source_terrain_layers,source_terrain_repeat,uv,layer,lod).rgb;
    let height=textureSampleLevel(source_terrain_heights,source_terrain_repeat,uv,layer,lod).r;
    let boost=clamp(height+clamp(2.0*weight-1.0,0.0,1.0),0.0,1.0);
    let blend=source_terrain.blend[layer];
    let alpha=min(pow(max(weight*boost*(1.0+blend.x),0.0001),blend.y),1.0);
    let layered=clamp(baked+(detail-vec3<f32>(0.5))*config.w,vec3<f32>(0.0),vec3<f32>(1.0));
    color=mix(color,layered,alpha);
    let normal=textureSampleLevel(source_terrain_normals,source_terrain_repeat,uv,layer,lod).rg*2.0-vec2<f32>(1.0);
    terrain_normal_xy=mix(terrain_normal_xy,normal,alpha);
  }
  return color;
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
    // The baked atlas arrives decoded; the game blends it display encoded and
    // squares the result back to linear.
    let display=blended_terrain(input,undo_srgb(base.rgb));
    base=vec4<f32>(display*display,base.a);
  } else if (authored.layers.y<0.5) {
    let detail=textureSample(terrain_detail,tex_sampler,input.uv);
    signed_detail=(detail.rgb-vec3<f32>(128.0/255.0))*detail.a;
  }
  // BLENDLAYER: vertex paint times the mask, shaped as the terrain blend is.
  let layer_sample=textureSample(blend_layer,tex_sampler,input.uv*authored.blend.y);
  let mask_sample=textureSample(blend_mask,tex_sampler,input.uv*authored.blend.z).r;
  if (authored.layers.z>0.5) {
    let weight=input.paint*mask_sample*(1.0+authored.layers.w);
    let amount=min(pow(max(weight,0.0001),authored.blend.x),1.0);
    base=vec4<f32>(mix(base.rgb,layer_sample.rgb,amount),base.a);
  }
  // The game's Detail map: green and alpha bend the normal, red scales diffuse,
  // blue scales gloss.
  let detail_map=textureSample(terrain_detail,tex_sampler,input.uv*authored.detail.xy);
  var detail_gloss=1.0;
  var detail_bend=vec2<f32>(0.0);
  if (authored.layers.y>0.5) {
    base=vec4<f32>(base.rgb*max(1.0+(detail_map.r*2.0-1.0)*authored.detail.w,0.0),base.a);
    detail_bend=(detail_map.ga*2.0-vec2<f32>(1.0))*authored.detail.z;
    detail_gloss=1.0+(detail_map.b*2.0-1.0)*authored.layers.x;
  }
  base=vec4<f32>(clamp(base.rgb+signed_detail,vec3<f32>(0.0),vec3<f32>(1.0))*authored.clouds.rgb*sun_shadows.control.w,base.a);
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
  let geometric=n;
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
  let plain_xy=encoded*2.0-vec2<f32>(1.0);
  let tangent_z=sqrt(max(0.0,1.0-dot(plain_xy,plain_xy)));
  let xy=plain_xy+detail_bend;
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
  // Blended water sheets are two sided, so the sun lights either face.
  let lambert=mix(max(dot(n,sun_direction),0.0),abs(dot(n,sun_direction)),blended_surface);
  let visibility=sun_visibility(input.world,normalize(input.normal+vec3<f32>(0.0,0.0,0.0001)),lambert);
  // A bump cannot catch sun on a face that is turned away from it.
  let facing_sun=mix(clamp(dot(geometric,sun_direction)*8.0+0.5,0.0,1.0),1.0,blended_surface);
  let sun=light.sun_color.rgb*(light.sun.w*lambert*visibility*facing_sun);
  // Source-authored specular reflectance and gloss are separate channels.
  // A missing specular map is black (never invent shiny materials).
  let reflectance=textureSample(specular_map,tex_sampler,input.uv).rgb*authored.specular.rgb;
  // A ddna normal map carries smoothness in alpha; other formats read as 1.
  let gloss=clamp(textureSample(gloss_map,tex_sampler,input.uv).r*textureSample(normal_map,tex_sampler,input.uv).a*detail_gloss,0.0,1.0);
  let view_direction=normalize(camera.origin.xyz-input.world);
  let half_direction=normalize(sun_direction+view_direction);
  let exponent=mix(8.0,120.0,clamp(gloss,0.0,1.0));
  let highlight=pow(max(dot(n,half_direction),0.0),exponent);
  let specular=reflectance*light.sun_color.rgb*(light.sun.w*0.6*highlight*visibility);
  // Reflect the map's baked probe. Only materials with authored specular do.
  // The game reads a BRDF table here; this is the common fit of that table, so
  // rough surfaces do not mirror the probe at grazing angles.
  let no_v=max(dot(n,view_direction),0.0);
  let fit=(1.0-clamp(gloss,0.0,1.0))*vec4<f32>(-1.0,-0.0275,-0.572,0.022)+vec4<f32>(1.0,0.0425,1.04,-0.04);
  let fit_a=min(fit.x*fit.x,pow(2.0,-9.28*no_v))*fit.x+fit.y;
  let fresnel=reflectance*(-1.04*fit_a+fit.z)+vec3<f32>(1.04*fit_a+fit.w);
  let has_specular=step(0.001,max(reflectance.r,max(reflectance.g,reflectance.b)));
  let reflect_direction=reflect(-view_direction,n);
  let reflect_lod=(1.0-clamp(gloss,0.0,1.0))*6.0;
  let probes=probe_light(input.world,n,reflect_direction,reflect_lod);
  let ambient_light=probes.ambient;
  let environment=probes.environment;
  let raw_reflected=min(environment,vec3<f32>(1.0))*fresnel*has_specular;
  // Blended water reads paler than the warm probe suggests; calibrated by eye.
  let reflected_luma=dot(raw_reflected,vec3<f32>(0.2126,0.7152,0.0722));
  let reflected=mix(raw_reflected,vec3<f32>(reflected_luma)*0.8,blended_surface*0.7);
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
    // The game's falloff: inverse square from the bulb, faded out at the radius.
    let edge=clamp((radius-dist)/radius*5.0,0.0,1.0);
    let bulb=max(source.bulb.x,0.001);
    let spread=max(dist-bulb,0.0)/bulb+1.0;
    var attenuation=edge*edge/(spread*spread);
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
  return vec4<f32>(base.rgb*(ambient_light*authored.clouds.w+sun)+specular+reflected+local_radiance+preview_glow,base.a);
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
    // The game's sun-side lobe: grows with distance, falls off as a power of
    // two away from the sun. How its authored size maps to the falloff is not
    // in the shader, so that scale is calibrated.
    let reach=clamp(distance*fog.params.y,0.0,1.0);
    let falloff=(1.0-fog.radial.w)*16.0;
    let toward_sun=dot(to_fragment/max(distance,0.001),normalize(light.sun.xyz));
    let lobe=pow(reach,1.0/fog.top.w)*pow(2.0,(toward_sun-1.0)*falloff);
    haze+=fog.radial.rgb*lobe;
    // The game's density term. Its 0.01 scale is inferred from the engine's usage.
    let amount=(1.0-pow(2.0,-distance*fog.ramp.w*0.01))*ramp;
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

// The game's Waterfall shader. Its refraction lerp is folded into premultiplied
// blending, so only the screen bend of the water behind is absent.
const WATERFALL_TIME_SCALE:f32=0.02; // The game's animation clock rate is not in its shaders.
@fragment
fn fs_waterfall(input:Output)->@location(0) vec4<f32> {
  let scroll=authored.waterfall_scroll;
  let surface=authored.waterfall_surface;
  let blend=authored.waterfall_blend;
  let time=camera.origin.w*WATERFALL_TIME_SCALE;
  let uv0=vec2<f32>(input.uv.x*scroll.x+time*scroll.z,input.uv.y*scroll.x);
  let uv1=vec2<f32>(input.uv.x*scroll.y+time*scroll.w,input.uv.y*scroll.y);
  let bump0=textureSample(normal_map,tex_sampler,uv0).rg*2.0-vec2<f32>(1.0);
  let bump1=textureSample(normal_map,tex_sampler,uv1).rg*2.0-vec2<f32>(1.0);
  let tangent_normal=normalize(vec3<f32>(bump0*surface.x+bump1*surface.y,
    sqrt(clamp(1.0-dot(bump0,bump0),0.0,1.0))));
  var n=normalize(input.normal+vec3<f32>(0.0,0.0,0.0001));
  let uv_dx=dpdx(input.uv);
  let uv_dy=dpdy(input.uv);
  let determinant=uv_dx.x*uv_dy.y-uv_dx.y*uv_dy.x;
  if (abs(determinant)>0.000001) {
    let raw=(dpdx(input.world)*uv_dy.y-dpdy(input.world)*uv_dx.y)/determinant;
    let tangent=raw-n*dot(n,raw);
    if (dot(tangent,tangent)>0.000001) {
      let t=normalize(tangent);
      n=normalize(t*tangent_normal.x+cross(n,t)*tangent_normal.y+n*tangent_normal.z);
    }
  }
  let view=normalize(camera.origin.xyz-input.world);
  let probes=probe_light(input.world,n,reflect(-view,n),0.0);
  let fresnel=clamp(surface.w+(1.0-surface.w)*pow(1.0-dot(view,n),5.0),0.0,1.0);
  let deform=tangent_normal.xy*blend.y;
  let foam=textureSample(albedo,tex_sampler,uv0+deform)+textureSample(albedo,tex_sampler,uv1+deform);
  let alpha=clamp(foam.a*blend.x*0.5,0.0,1.0);
  // The game's object ambient input also carries sky and sun; this share is calibrated.
  let incoming=probes.ambient+light.sun_color.rgb*light.sun.w*0.8;
  let lit=foam.rgb*incoming*(n.z*0.25+0.75)*authored.clouds.rgb*blend.z*0.5;
  let fogged=distance_fog(input,vec4<f32>(lit+probes.environment*surface.z*fresnel,1.0)).rgb;
  return vec4<f32>(fogged*alpha,alpha*fresnel);
}

// The mirrored view draws only what is above the water surface.
@fragment
fn fs_opaque_reflected(input:Output)->@location(0) vec4<f32> {
  if (input.world.z<camera.reflection_height.x) { discard; }
  let fogged=distance_fog(input,shade(input,true));
  return vec4<f32>(authored_scene_linear(fogged.rgb),1.0);
}

@fragment
fn fs_foliage_reflected(input:Output)->@location(0) vec4<f32> {
  if (input.world.z<camera.reflection_height.x) { discard; }
  if (sample_source_albedo(input.uv).a < authored.cutoff.x) { discard; }
  let fogged=distance_fog(input,shade(input,false));
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
  blended_surface=1.0;
  let color=shade(input,false);
  let fogged=distance_fog(input,color);
  let alpha=textureSample(authored_opacity,tex_sampler,input.uv).r;
  return vec4<f32>(authored_scene_linear(fogged.rgb),color.a*alpha);
}