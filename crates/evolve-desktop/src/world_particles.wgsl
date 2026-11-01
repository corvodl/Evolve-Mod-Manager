// Authored particle emitters. Each particle replays a fixed loop from its seed,
// so nothing is simulated on the CPU. The lighting follows the game's particle
// shader; the fade curve is a shared approximation of the authored curves.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
struct TimeOfDayLight { sun:vec4<f32>, ambient:vec4<f32>, sun_color:vec4<f32>, }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<uniform> light:TimeOfDayLight;
@group(0) @binding(2) var sprite:texture_2d<f32>;
@group(0) @binding(3) var sprite_sampler:sampler;
@group(1) @binding(2) var scene_depth:texture_depth_2d;
struct SunShadows { matrices:array<mat4x4<f32>,4>, control:vec4<f32>, }
@group(1) @binding(4) var<uniform> sun_shadows:SunShadows;
@group(1) @binding(5) var shadow_depth:texture_depth_2d_array;
@group(1) @binding(6) var shadow_compare:sampler_comparison;

struct Output {
  @builtin(position) position:vec4<f32>,
  @location(0) uv:vec2<f32>,
  @location(1) color:vec4<f32>,
  @location(2) view_depth:f32,
  @location(3) additive:f32,
  @location(4) sun:vec3<f32>,
  @location(5) world:vec3<f32>,
}

@vertex
fn vs_particle(
  @builtin(vertex_index) vertex:u32,
  @location(0) origin_phase:vec4<f32>,
  @location(1) velocity_life:vec4<f32>,
  @location(2) shape:vec4<f32>,   // birth size, full size, gravity scale, alpha
  @location(3) tint_spin:vec4<f32>,
  @location(4) tiles:vec4<f32>,   // tiles across, tiles down, frames per second, first tile
  @location(5) lighting:vec4<f32>, // diffuse, emissive, wrap radius, additive
  @location(6) stretch:vec4<f32>,  // scale X, scale Y, follows its motion
  @location(7) curve:vec4<f32>,    // alpha rise start and end, fall start and end
)->Output {
  let life=velocity_life.w;
  let age=fract(camera.origin.w/life+origin_phase.w)*life;
  let t=age/life;
  var center=origin_phase.xyz+velocity_life.xyz*age;
  center.z-=4.905*shape.z*age*age;
  if (lighting.z>0.0) {
    // Rain repeats around the camera and falls from just above it.
    let span=lighting.z*2.0;
    center=vec3<f32>(
      camera.origin.x+(fract((origin_phase.x-camera.origin.x)/span)-0.5)*span,
      camera.origin.y+(fract((origin_phase.y-camera.origin.y)/span)-0.5)*span,
      camera.origin.z+(0.5-t)*lighting.z);
  }
  let corners=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),
    vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
  let corner=corners[vertex%6u];
  let angle=tint_spin.w;
  let spun=vec2<f32>(corner.x*cos(angle)-corner.y*sin(angle),corner.x*sin(angle)+corner.y*cos(angle));
  let size=mix(shape.x,shape.y,t);
  let motion=velocity_life.xyz-vec3<f32>(0.0,0.0,9.81*shape.z*age);
  var world=center+(camera.right.xyz*spun.x*stretch.x+camera.up.xyz*spun.y*stretch.y)*size;
  if (stretch.z>0.5 && dot(motion,motion)>0.000001) {
    // A falling sheet: long along its motion, wide across the view.
    let along=normalize(motion);
    let across=cross(along,normalize(center-camera.origin.xyz));
    if (dot(across,across)>0.000001) {
      world=center+(normalize(across)*corner.x*stretch.x+along*corner.y*stretch.y)*size;
    }
  }
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.position=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  let total=max(tiles.x*tiles.y,1.0);
  let frame=(tiles.w+floor(age*tiles.z))%total;
  let cell=vec2<f32>(frame%max(tiles.x,1.0),floor(frame/max(tiles.x,1.0)));
  out.uv=(cell+corner*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5))/max(tiles.xy,vec2<f32>(1.0));
  let fade=clamp((t-curve.x)/max(curve.y-curve.x,0.001),0.0,1.0)
    *clamp((curve.w-t)/max(curve.w-curve.z,0.001),0.0,1.0);
  out.sun=tint_spin.rgb*light.sun_color.rgb*light.sun.w*max(normalize(light.sun.xyz).z,0.0)*0.5*lighting.x;
  out.world=world;
  out.color=vec4<f32>(tint_spin.rgb*(light.ambient.rgb*lighting.x+vec3<f32>(lighting.y)),shape.w*fade);
  out.view_depth=d;
  out.additive=lighting.w;
  return out;
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

@fragment
fn fs_particle(input:Output)->@location(0) vec4<f32> {
  let source=textureSample(sprite,sprite_sampler,input.uv);
  let z=textureLoad(scene_depth,vec2<i32>(input.position.xy),0);
  let behind=camera.forward.w/max(1.0-z,0.000001)-input.view_depth;
  let soft=clamp(behind*0.5,0.0,1.0);
  let alpha=source.a*input.color.a*soft;
  let lit=input.color.rgb+input.sun*sun_visibility(input.world);
  return vec4<f32>(source.rgb*lit*alpha,alpha*(1.0-input.additive));
}
