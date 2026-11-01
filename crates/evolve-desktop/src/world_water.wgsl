// Authored WaterVolume surfaces, following the game's compiled water shader:
// layered normals, planar reflection, refraction, depth absorption, soft edges
// and its two-lobe sun highlight. The engine generates its water normal map at
// runtime and the decal permutation is not in that shader, so `water_slope` and
// the decal blend are calibrated rather than decoded.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
struct TimeOfDayLight { sun:vec4<f32>, ambient:vec4<f32>, sun_color:vec4<f32>, }
struct FogPreview {
  color:vec4<f32>, params:vec4<f32>, radial:vec4<f32>, top:vec4<f32>, ramp:vec4<f32>,
}
struct WaterParams {
  tint:vec4<f32>,     // authored Diffuse, decal strength
  tiling:vec4<f32>,   // Tilling, DetailTilling, DecalTilling, NormalsScale
  surface:vec4<f32>,  // DetailNormalsScale, SoftIntersectionFactor, reserved
  gloss:vec4<f32>,    // GlossMapTilling, GlossMapScale, GlossMapBias, has map
  fog:vec4<f32>,      // authored fog color, density
  specular:vec4<f32>, // authored Specular
}
struct SunShadows {
  matrices:array<mat4x4<f32>,4>,
  control:vec4<f32>,
  ambient_cube:array<vec4<f32>,6>,
  probe_tint:vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<uniform> light:TimeOfDayLight;
@group(0) @binding(2) var<uniform> fog:FogPreview;
@group(0) @binding(3) var<uniform> hdr:vec4<f32>;
@group(0) @binding(4) var<uniform> water:WaterParams;
@group(0) @binding(5) var decal:texture_2d<f32>;
@group(0) @binding(6) var water_sampler:sampler;
@group(0) @binding(7) var gloss_map:texture_2d<f32>;
@group(1) @binding(0) var<uniform> sun_shadows:SunShadows;
@group(1) @binding(1) var shadow_depth:texture_depth_2d_array;
@group(1) @binding(2) var shadow_compare:sampler_comparison;
@group(2) @binding(0) var reflection:texture_2d<f32>;
@group(2) @binding(1) var refraction:texture_2d<f32>;
@group(2) @binding(2) var scene_depth:texture_depth_2d;
@group(2) @binding(3) var screen_sampler:sampler;

struct Output {
  @builtin(position) position:vec4<f32>,
  @location(0) world:vec3<f32>,
  @location(1) clip:vec4<f32>,
}

@vertex
fn vs_water(@location(0) position:vec3<f32>)->Output {
  let delta=position-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.clip=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  out.position=out.clip;
  out.world=position;
  return out;
}

// Stands in for the engine's runtime water normals: signed tangent slope.
fn water_slope(uv:vec2<f32>)->vec2<f32> {
  let time=camera.origin.w;
  let p=uv*6.2831853;
  var slope=vec2<f32>(0.0);
  let directions=array<vec2<f32>,6>(
    vec2<f32>(0.80,0.60),vec2<f32>(-0.55,0.83),vec2<f32>(0.26,-0.97),
    vec2<f32>(-0.92,-0.39),vec2<f32>(0.64,-0.77),vec2<f32>(-0.17,0.98));
  var frequency=1.0;
  var amplitude=0.16;
  for (var i=0;i<6;i=i+1) {
    let phase=dot(directions[i],p)*frequency+time*(0.7+f32(i)*0.31);
    slope+=directions[i]*cos(phase+sin(dot(directions[(i+1)%6],p)*frequency*0.37))*amplitude;
    frequency*=1.73;
    amplitude*=0.72;
  }
  return slope;
}

fn sun_visibility(world:vec3<f32>)->f32 {
  if (sun_shadows.control.x<0.5) { return 1.0; }
  for (var index:i32=0;index<4;index=index+1) {
    let projected=sun_shadows.matrices[index]*vec4<f32>(world,1.0);
    if (any(abs(projected.xy)>vec2<f32>(0.985))) { continue; }
    let uv=vec2<f32>(projected.x*0.5+0.5,0.5-projected.y*0.5);
    return textureSampleCompareLevel(shadow_depth,shadow_compare,uv,index,projected.z);
  }
  return 1.0;
}

fn apply_fog(world:vec3<f32>,color:vec3<f32>)->vec3<f32> {
  if (fog.params.x<0.5) { return color; }
  let to_fragment=world-camera.origin.xyz;
  let distance=length(to_fragment);
  let ramp_span=max(fog.ramp.y-fog.ramp.x,0.01);
  var ramp=clamp((distance-fog.ramp.x)/ramp_span,0.0,1.0);
  ramp=mix(1.0,ramp*(2.0-ramp),fog.ramp.z);
  let amount=(1.0-exp(-distance*fog.params.y*fog.params.z*fog.ramp.w*15.0))*ramp;
  let sharpness=1.0/max(1.0-fog.radial.w,0.001);
  let toward=dot(to_fragment/max(distance,0.001),normalize(light.sun.xyz));
  let reach=clamp(distance*fog.params.y,0.0,1.0);
  let haze=fog.color.rgb+fog.radial.rgb*pow(reach,fog.top.w)*pow(2.0,(toward-1.0)*sharpness);
  return mix(color,haze,clamp(amount,0.0,1.0));
}

// Linear view depth behind a screen position; the depth buffer is z=(d-near)/d.
fn depth_at(uv:vec2<f32>)->f32 {
  let size=vec2<f32>(textureDimensions(scene_depth));
  let texel=vec2<i32>(clamp(uv*size,vec2<f32>(0.0),size-vec2<f32>(1.0)));
  let z=textureLoad(scene_depth,texel,0);
  return camera.forward.w/max(1.0-z,0.000001);
}

@fragment
fn fs_water(input:Output)->@location(0) vec4<f32> {
  let view_ray=camera.origin.xyz-input.world;
  let view=view_ray/max(length(view_ray),0.001);
  let view_depth=input.clip.w;
  let size=vec2<f32>(textureDimensions(refraction));
  let screen=input.position.xy/size;
  let ndc=input.clip.xy/view_depth;
  let reflection_uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);

  let uv=input.world.xy*water.tiling.x;
  let detail_uv=uv*water.tiling.y*2.0;
  // Two parallax steps through the normal map, as the game does.
  var offset=water_slope(uv*0.25)*0.075;
  offset+=water_slope(uv+offset)*0.075;
  let slope_a=water_slope(uv*0.25+offset);
  let slope_b=water_slope(uv+offset);
  let slope_detail=water_slope(detail_uv+offset);
  var slope=(slope_a+slope_b+slope_detail*water.surface.x)*water.tiling.w;
  let n=normalize(vec3<f32>(slope,2.0));

  let gloss_uv=uv*water.gloss.x+slope*0.05;
  var gloss=clamp(water.gloss.z,0.0,1.0);
  if (water.gloss.w>0.5) {
    gloss=clamp((textureSample(gloss_map,water_sampler,gloss_uv).r
      +textureSample(gloss_map,water_sampler,gloss_uv*8.0).r)*water.gloss.y+water.gloss.z,0.0,1.0);
  }

  let behind=depth_at(screen)-view_depth;
  let soft=clamp(behind*water.surface.y,0.0,1.0);
  let distance_fade=1.0-sqrt(clamp(view_depth*0.02,0.0,1.0));

  let bent=normalize(mix(vec3<f32>(0.0,0.0,1.0),n,0.3));
  let reflect_shift=bent.xy*soft*distance_fade*0.15;
  let mirrored=textureSampleLevel(reflection,screen_sampler,
    clamp(reflection_uv+reflect_shift,vec2<f32>(0.001),vec2<f32>(0.999)),0.0).rgb;

  // Refraction splits slightly by channel and rejects anything in front.
  let shift=n.xy*0.1*soft*distance_fade;
  let accepted=select(vec2<f32>(0.0),shift,depth_at(screen+shift)>view_depth);
  let refracted=vec3<f32>(
    textureSampleLevel(refraction,screen_sampler,screen+accepted*1.025,0.0).r,
    textureSampleLevel(refraction,screen_sampler,screen+accepted,0.0).g,
    textureSampleLevel(refraction,screen_sampler,screen+accepted*0.985,0.0).b);
  let scene_behind=max(depth_at(screen+accepted)-view_depth,0.0);
  let facing_camera=max(-dot(view,camera.forward.xyz),0.01);
  let clarity=min(pow(2.0,-water.fog.w*scene_behind/facing_camera),1.0);

  let sun_direction=normalize(light.sun.xyz);
  let visibility=sun_visibility(input.world+vec3<f32>(0.0,0.0,0.05));
  let sun_radiance=light.sun_color.rgb*light.sun.w*visibility;
  let ambient=sun_shadows.ambient_cube[4].rgb+light.ambient.rgb*(1.0-sun_shadows.control.z);
  let fog_color=water.fog.rgb*(ambient+sun_radiance*max(sun_direction.z,0.0));
  let body=mix(fog_color,refracted,clarity);

  let reflected=reflect(-view,n);
  let toward_sun=max(dot(reflected,sun_direction),0.000001);
  let highlight=(0.31831+162.338*gloss)*pow(toward_sun,1020.0*gloss)
    +(0.31831+20.2923*gloss)*pow(toward_sun,127.5*gloss);
  let sun_light=highlight*max(dot(n,sun_direction),0.0)*sun_radiance*water.specular.rgb;

  let facing=1.0-clamp(dot(view,n),0.0,1.0);
  let fresnel=(0.05+0.95*pow(facing,5.0))*clamp(view_depth-0.33,0.0,1.0)*soft;
  var color=mix(body,mirrored+sun_light,fresnel);

  let drift=textureSample(decal,water_sampler,input.world.xy*water.tiling.z);
  let decal_light=ambient+light.sun_color.rgb*0.35*visibility;
  color=mix(color,drift.rgb*decal_light,clamp(drift.a*water.tint.w,0.0,1.0)*0.25*soft);
  return vec4<f32>(apply_fog(input.world,color),soft);
}
