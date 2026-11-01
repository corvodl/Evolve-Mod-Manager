// Authored deferred decals: a box around each decal reads the scene depth and
// projects its texture along the box's Z axis onto whatever lies inside. The
// game lights them in its deferred pass; here the lit scene behind each pixel
// stands in for that light, so the albedo scale and view fade are calibrated.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(2) var decal_map:texture_2d<f32>;
@group(0) @binding(3) var decal_sampler:sampler;
@group(1) @binding(1) var scene_color:texture_2d<f32>;
@group(1) @binding(2) var scene_depth:texture_depth_2d;
@group(1) @binding(3) var screen_sampler:sampler;

struct Output {
  @builtin(position) position:vec4<f32>,
  @location(0) clip:vec4<f32>,
  @location(1) @interpolate(flat) axis_x:vec4<f32>,
  @location(2) @interpolate(flat) axis_y:vec4<f32>,
  @location(3) @interpolate(flat) axis_z:vec4<f32>,
  @location(4) @interpolate(flat) center:vec4<f32>,
  @location(5) @interpolate(flat) tint:vec4<f32>,
}

@vertex
fn vs_decal(
  @builtin(vertex_index) vertex:u32,
  @location(0) axis_x:vec4<f32>, // world axis, half size
  @location(1) axis_y:vec4<f32>,
  @location(2) axis_z:vec4<f32>,
  @location(3) center:vec4<f32>, // world position, DecalFalloff
  @location(4) tint:vec4<f32>,   // authored Diffuse, opacity
)->Output {
  let indices=array<u32,36>(0u,1u,2u,2u,1u,3u,4u,6u,5u,5u,6u,7u,0u,4u,1u,1u,4u,5u,
    2u,3u,6u,6u,3u,7u,0u,2u,4u,4u,2u,6u,1u,5u,3u,3u,5u,7u);
  let i=indices[vertex%36u];
  let corner=vec3<f32>(f32(i&1u)*2.0-1.0,f32((i>>1u)&1u)*2.0-1.0,f32((i>>2u)&1u)*2.0-1.0);
  let world=center.xyz+axis_x.xyz*axis_x.w*corner.x+axis_y.xyz*axis_y.w*corner.y+axis_z.xyz*axis_z.w*corner.z;
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:Output;
  out.clip=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  out.position=out.clip;
  out.axis_x=axis_x;
  out.axis_y=axis_y;
  out.axis_z=axis_z;
  out.center=center;
  out.tint=tint;
  return out;
}

// Roughly what surfaces reflect, to turn the scene color into incoming light.
const TYPICAL_ALBEDO:f32=0.3;

@fragment
fn fs_decal(input:Output)->@location(0) vec4<f32> {
  // The scene point under this pixel, from its depth.
  let z=textureLoad(scene_depth,vec2<i32>(input.position.xy),0);
  let view_depth=camera.forward.w/max(1.0-z,0.000001);
  let ndc=input.clip.xy/input.clip.w;
  let ray=camera.forward.xyz+camera.right.xyz*camera.right.w*ndc.x+camera.up.xyz*camera.up.w*ndc.y;
  let world=camera.origin.xyz+ray*view_depth;
  let offset=world-input.center.xyz;
  let local=vec3<f32>(dot(offset,input.axis_x.xyz)/input.axis_x.w,
    dot(offset,input.axis_y.xyz)/input.axis_y.w,
    dot(offset,input.axis_z.xyz)/input.axis_z.w);
  if (any(abs(local)>vec3<f32>(1.0))) { discard; }
  let n=normalize(cross(dpdx(world),dpdy(world)));
  let uv=vec2<f32>(local.x,-local.y)*0.5+vec2<f32>(0.5);
  let source=textureSampleGrad(decal_map,decal_sampler,uv,dpdx(uv),dpdy(uv));
  // Fade where the surface turns away from the projection, and with depth.
  let facing=clamp(abs(dot(n,input.axis_z.xyz))*3.0-0.5,0.0,1.0);
  let edge=clamp((1.0-abs(local.z))*4.0,0.0,1.0);
  let view_fade=clamp((220.0-view_depth)/60.0,0.0,1.0);
  let alpha=pow(max(source.a,0.0001),input.center.w)*input.tint.a*facing*edge*view_fade;
  if (alpha<=0.002) { discard; }
  let behind=textureSampleLevel(scene_color,screen_sampler,input.position.xy/vec2<f32>(textureDimensions(scene_color)),0.0).rgb;
  return vec4<f32>(source.rgb*input.tint.rgb*behind/TYPICAL_ALBEDO,alpha);
}
