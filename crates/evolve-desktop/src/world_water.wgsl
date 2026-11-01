// Authored WaterVolume surfaces. Inputs are the map's polygons, material tint,
// tiling and probe; the wave, reflection and absorption math is calibrated to
// in-game captures and is not decoded engine shader code.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
struct TimeOfDayLight { sun:vec4<f32>, ambient:vec4<f32>, sun_color:vec4<f32>, }
struct FogPreview {
  color:vec4<f32>, params:vec4<f32>, radial:vec4<f32>, top:vec4<f32>, ramp:vec4<f32>,
}
struct WaterParams {
  tint:vec4<f32>,   // authored Diffuse, decal strength
  tiling:vec4<f32>, // Tilling, DetailTilling, DecalTilling, normals scale
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
@group(1) @binding(0) var<uniform> sun_shadows:SunShadows;
@group(1) @binding(1) var shadow_depth:texture_depth_2d_array;
@group(1) @binding(2) var shadow_compare:sampler_comparison;
@group(1) @binding(3) var probe_cube:texture_cube<f32>;
@group(1) @binding(4) var probe_sampler:sampler;

struct Output { @builtin(position) clip:vec4<f32>, @location(0) world:vec3<f32>, }

@vertex
fn vs_water(@location(0) position:vec3<f32>)->Output {
  let delta=position-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.clip=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  out.world=position;
  return out;
}

// Slope of a sum of crossing sine ripples, so the surface needs no normal map.
fn ripple_slope(p:vec2<f32>,time:f32)->vec2<f32> {
  var slope=vec2<f32>(0.0);
  let directions=array<vec2<f32>,6>(
    vec2<f32>(0.80,0.60),vec2<f32>(-0.55,0.83),vec2<f32>(0.26,-0.97),
    vec2<f32>(-0.92,-0.39),vec2<f32>(0.64,-0.77),vec2<f32>(-0.17,0.98));
  var frequency=1.0;
  var amplitude=1.0;
  for (var i=0;i<6;i=i+1) {
    let phase=dot(directions[i],p)*frequency+time*(0.7+f32(i)*0.31);
    slope+=directions[i]*cos(phase+sin(dot(directions[(i+1)%6],p)*frequency*0.37))*amplitude;
    frequency*=1.73;
    amplitude*=0.62;
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
  let amount=(1.0-exp(-distance*fog.params.y*fog.params.z*fog.ramp.w))*ramp;
  let sharpness=1.0/max(1.0-fog.radial.w,0.001);
  let toward=dot(to_fragment/max(distance,0.001),normalize(light.sun.xyz));
  let reach=clamp(distance*fog.params.y,0.0,1.0);
  let haze=fog.color.rgb+fog.radial.rgb*pow(reach,fog.top.w)*pow(2.0,(toward-1.0)*sharpness);
  return mix(color,haze,clamp(amount,0.0,1.0));
}

@fragment
fn fs_water(input:Output)->@location(0) vec4<f32> {
  let time=camera.origin.w;
  let view_ray=camera.origin.xyz-input.world;
  let distance=length(view_ray);
  let view=view_ray/max(distance,0.001);
  let large=ripple_slope(input.world.xy*water.tiling.x*2.0,time);
  let fine=ripple_slope(input.world.xy*water.tiling.y*0.6,time*1.6);
  // Fine ripples fade with distance so they do not shimmer into noise.
  let fade=1.0/(1.0+distance*0.03);
  let slope=(large*0.07+fine*0.05*fade)*water.tiling.w;
  let n=normalize(vec3<f32>(-slope.x,-slope.y,1.0));
  var reflected=reflect(-view,n);
  reflected.z=abs(reflected.z);
  let env=textureSampleLevel(probe_cube,probe_sampler,reflected,1.0).rgb*sun_shadows.probe_tint.rgb;
  let facing=max(dot(n,view),0.0);
  let fresnel=0.02+0.98*pow(1.0-facing,5.0);
  let sun_direction=normalize(light.sun.xyz);
  let half_way=normalize(sun_direction+view);
  let visibility=sun_visibility(input.world+vec3<f32>(0.0,0.0,0.05));
  let glint=pow(max(dot(n,half_way),0.0),600.0)*light.sun.w*visibility;
  let ambient=sun_shadows.ambient_cube[4].rgb+light.ambient.rgb*(1.0-sun_shadows.control.z);
  var body=water.tint.rgb*(ambient+light.sun_color.rgb*max(sun_direction.z,0.0)*visibility);
  let surface=textureSample(decal,water_sampler,input.world.xy*water.tiling.z);
  body=mix(body,surface.rgb*(ambient+light.sun_color.rgb*0.35*visibility),
    clamp(surface.a*water.tint.w,0.0,1.0)*0.25);
  let color=mix(body,env,fresnel)+light.sun_color.rgb*glint*6.0;
  let fogged=apply_fog(input.world,color)*hdr.x*(1.0+0.05*hdr.y);
  // Opaque looking murk, thinning a little where the view drops steeply.
  // Clear from above so the bed shows; a mirror at grazing angles.
  return vec4<f32>(fogged,mix(0.38,0.97,clamp(fresnel*2.5+distance*0.002,0.0,1.0)));
}
