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

struct Output {
  @builtin(position) position:vec4<f32>,
  @location(0) uv:vec2<f32>,
  @location(1) color:vec4<f32>,
  @location(2) view_depth:f32,
  @location(3) additive:f32,
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
  let world=center+(camera.right.xyz*spun.x+camera.up.xyz*spun.y)*size;
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.position=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  let total=max(tiles.x*tiles.y,1.0);
  let frame=(tiles.w+floor(age*tiles.z))%total;
  let cell=vec2<f32>(frame%max(tiles.x,1.0),floor(frame/max(tiles.x,1.0)));
  out.uv=(cell+corner*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5))/max(tiles.xy,vec2<f32>(1.0));
  let fade=clamp(t/0.16,0.0,1.0)*clamp((1.0-t)/0.4,0.0,1.0);
  let sun=light.sun_color.rgb*light.sun.w*max(normalize(light.sun.xyz).z,0.0)*0.5;
  out.color=vec4<f32>(tint_spin.rgb*((light.ambient.rgb+sun)*lighting.x+vec3<f32>(lighting.y)),shape.w*fade);
  out.view_depth=d;
  out.additive=lighting.w;
  return out;
}

@fragment
fn fs_particle(input:Output)->@location(0) vec4<f32> {
  let source=textureSample(sprite,sprite_sampler,input.uv);
  let z=textureLoad(scene_depth,vec2<i32>(input.position.xy),0);
  let behind=camera.forward.w/max(1.0-z,0.000001)-input.view_depth;
  let soft=clamp(behind*0.5,0.0,1.0);
  let alpha=source.a*input.color.a*soft;
  return vec4<f32>(source.rgb*input.color.rgb*alpha,alpha*(1.0-input.additive));
}
