// Read-only Level Editor light-point corona preview.
// Source color, intensity, radius and world placement come from the map's
// validated light entities. This is a deliberately bounded editor indication,
// not an Evolve LensOptics optical flare or a particle simulation.
struct Camera {
  origin:vec4<f32>,
  forward:vec4<f32>,
  right:vec4<f32>,
  up:vec4<f32>,
}
struct LightPoint {
  position_radius:vec4<f32>,
  color_intensity:vec4<f32>,
}
struct LightPoints { data:array<LightPoint>, }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<storage,read> sources:LightPoints;

struct Output {
  @builtin(position) clip:vec4<f32>,
  @location(0) uv:vec2<f32>,
  @location(1) tint:vec4<f32>,
}
@vertex
fn vs_light(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->Output {
  let point=sources.data[instance];
  let delta=point.position_radius.xyz-camera.origin.xyz;
  let forward=dot(delta,camera.forward.xyz);
  let right=dot(delta,camera.right.xyz);
  let up=dot(delta,camera.up.xyz);
  var result:Output;
  let offsets=array<vec2<f32>,6>(
     vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),
     vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0)
  );
  let quad=offsets[vertex];
  let safe_depth=max(forward,0.001);
  let radius=clamp(point.position_radius.w,0.1,50000.0);
  let half_size=clamp(0.008*radius/safe_depth,0.0015,0.012);
  let center=vec2<f32>(
     right/(safe_depth*camera.right.w),
     up/(safe_depth*camera.up.w)
  );
  let spread=vec2<f32>(half_size/max(camera.right.w,0.001),
                        half_size/max(camera.up.w,0.001));
  let ndc=center+quad*spread;
  // When behind the camera, place the triangle beyond clip space.
  if (forward<=0.05) {
    result.clip=vec4<f32>(2.0,2.0,2.0,1.0);
  } else {
    result.clip=vec4<f32>(ndc,0.99999,1.0);
  }
  result.uv=quad;
  let strength=clamp(point.color_intensity.a,0.0,16.0);
  result.tint=vec4<f32>(point.color_intensity.rgb,strength);
  return result;
}
@fragment
fn fs_light(input:Output)->@location(0) vec4<f32> {
  let r2=dot(input.uv,input.uv);
  if (r2>=1.0) {discard;}
  let falloff=pow(max(0.0,1.0-r2),2.0);
  let alpha=clamp(input.tint.a*0.10,0.0,0.42)*falloff;
  return vec4<f32>(input.tint.rgb,alpha);
}
