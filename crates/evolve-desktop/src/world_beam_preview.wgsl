// Stage-2 EDITOR preview of spot-light scattering, not native CryEngine
// raymarching. This is a depth-tested soft cone sheet anchored to original
// map spot positions, direction, cone half angle and light radius. It avoids
// the original opaque Beam CGF placeholder and does not invent a light.
struct Camera {
  origin:vec4<f32>,
  forward:vec4<f32>,
  right:vec4<f32>,
  up:vec4<f32>,
}
struct Spot {
  position_radius:vec4<f32>,
  direction_cone:vec4<f32>,
  color_strength:vec4<f32>,
}
struct Sources { data:array<Spot> }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<storage,read> sources:Sources;

struct Out {
  @builtin(position) clip:vec4<f32>,
  @location(0) field:vec2<f32>,
  @location(1) color:vec4<f32>,
}
@vertex
fn vs_beam(@builtin(instance_index) instance:u32,
           @builtin(vertex_index) vertex:u32)->Out {
  let spot=sources.data[instance];
  let quad=array<vec2<f32>,6>(
     vec2<f32>(-1.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0),
     vec2<f32>(-1.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
  let t=quad[vertex].y;
  let side=quad[vertex].x;
  let radius=clamp(spot.position_radius.w,0.01,1500.0);
  let cone_cos=clamp(spot.direction_cone.w,0.1,0.9999);
  let cone_tan=sqrt(max(1.0-cone_cos*cone_cos,0.0))/cone_cos;
  let width=radius*cone_tan*(0.05+0.95*t);
  let direction=normalize(spot.direction_cone.xyz);
  // A camera-facing billboard through the exact source and target.
  // Geometry has no opacity other than the analytic scattering profile.
  let world=spot.position_radius.xyz+direction*(radius*(0.02+0.98*t))
     +camera.right.xyz*(side*width);
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Out;
  if (d<=camera.forward.w+0.001) {
    out.clip=vec4<f32>(2.0,2.0,2.0,1.0);
  } else {
    out.clip=vec4<f32>(
      dot(delta,camera.right.xyz)/camera.right.w,
      dot(delta,camera.up.xyz)/camera.up.w,
      d-camera.forward.w,d);
  }
  out.field=vec2<f32>(side,t);
  out.color=spot.color_strength;
  return out;
}
@fragment
fn fs_beam(input:Out)->@location(0) vec4<f32> {
  let radial=pow(max(0.0,1.0-input.field.x*input.field.x),3.0);
  let longitudinal=smoothstep(0.0,0.08,input.field.y)
    *(1.0-smoothstep(0.45,1.0,input.field.y));
  let brightness=clamp(input.color.w,0.0,12.0);
  let alpha=clamp(0.025*brightness*radial*longitudinal,0.0,0.17);
  return vec4<f32>(max(input.color.rgb,vec3<f32>(0.0)),alpha);
}
