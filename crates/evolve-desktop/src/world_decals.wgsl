// Authored deferred decals: a box around each decal reads the scene depth and
// projects its texture along the box's Z axis onto whatever lies inside. The
// game lights them in its deferred pass; here the lit scene behind each pixel
// stands in for that light, so the albedo scale and view fade are calibrated.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(2) var decal_map:texture_2d<f32>;
@group(0) @binding(3) var decal_sampler:sampler;
struct TimeOfDayLight { sun:vec4<f32>, ambient:vec4<f32>, sun_color:vec4<f32>, }
@group(0) @binding(1) var<uniform> light:TimeOfDayLight;
@group(0) @binding(4) var road_normal:texture_2d<f32>;
@group(0) @binding(5) var<uniform> road_specular:vec4<f32>;
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

// The game's light pass: GGX over its smoothness, with Schlick Fresnel. A surface
// with no specular at all gets no grazing sheen either.
fn game_specular(n:vec3<f32>,to_view:vec3<f32>,to_light:vec3<f32>,f0:vec3<f32>,smoothness:f32)->vec3<f32> {
  let h=normalize(to_light+to_view);
  let no_l=max(dot(n,to_light),0.0);
  let no_v=max(dot(n,to_view),0.0);
  let no_h=max(dot(n,h),0.0);
  let rough=1.0-clamp(smoothness,0.0,1.0)*0.7;
  let rough2=rough*rough;
  let rough6=rough2*rough2*rough2;
  let alpha2=max(rough6*rough6,0.00001);
  let c=rough6*0.5+0.8;
  let k=c*c*0.5;
  let visibility=max((no_v*(1.0-k)+k)*(no_l*(1.0-k)+k),0.000001);
  let d=no_h*no_h*(alpha2-1.0)+1.0;
  let edge=1.0-max(dot(to_light,h),0.0);
  let fresnel=f0+(vec3<f32>(1.0)-f0)*edge*edge*edge*edge*edge;
  return fresnel*step(0.001,max(f0.r,max(f0.g,f0.b)))*(alpha2/(d*d)/visibility*0.25*no_l);
}
// Authored roads: a prism around each ribbon piece, projected only onto terrain
// pixels, which the opaque pass marks with half alpha. As with decals the scene
// stands in for light, here divided by what bare ground roughly reflects.
const ROAD_REACH:f32=1.5;
const GROUND_ALBEDO:f32=0.12;

struct RoadOutput {
  @builtin(position) position:vec4<f32>,
  @location(0) clip:vec4<f32>,
  @location(1) @interpolate(flat) left0:vec4<f32>,
  @location(2) @interpolate(flat) right0:vec4<f32>,
  @location(3) @interpolate(flat) left1:vec4<f32>,
  @location(4) @interpolate(flat) right1:vec4<f32>,
  @location(5) @interpolate(flat) tint:vec4<f32>,
}

@vertex
fn vs_road(
  @builtin(vertex_index) vertex:u32,
  @location(0) left0:vec4<f32>,  // world position, distance along the road in tiles
  @location(1) right0:vec4<f32>, // world position, end fade
  @location(2) left1:vec4<f32>,
  @location(3) right1:vec4<f32>,
  @location(4) tint:vec4<f32>,
)->RoadOutput {
  let indices=array<u32,36>(0u,1u,2u,2u,1u,3u,4u,6u,5u,5u,6u,7u,0u,4u,1u,1u,4u,5u,
    2u,3u,6u,6u,3u,7u,0u,2u,4u,4u,2u,6u,1u,5u,3u,3u,5u,7u);
  let i=indices[vertex%36u];
  let start=select(left0.xyz,right0.xyz,(i&1u)==1u);
  let end=select(left1.xyz,right1.xyz,(i&1u)==1u);
  var world=select(start,end,((i>>1u)&1u)==1u);
  world.z+=f32((i>>2u)&1u)*2.0*ROAD_REACH-ROAD_REACH;
  let delta=world-camera.origin.xyz;
  let d=dot(delta,camera.forward.xyz);
  var out:RoadOutput;
  out.clip=vec4<f32>(dot(delta,camera.right.xyz)/camera.right.w,
    dot(delta,camera.up.xyz)/camera.up.w,d-camera.forward.w,d);
  out.position=out.clip;
  out.left0=left0;
  out.right0=right0;
  out.left1=left1;
  out.right1=right1;
  out.tint=tint;
  return out;
}

// Where a point falls in a four sided piece: across it, then along it.
fn road_coords(p:vec2<f32>,a:vec2<f32>,b:vec2<f32>,c:vec2<f32>,d:vec2<f32>)->vec2<f32> {
  let e=b-a;
  let f=d-a;
  let g=a-b+c-d;
  let h=p-a;
  let k2=g.x*f.y-g.y*f.x;
  let k1=e.x*f.y-e.y*f.x+h.x*g.y-h.y*g.x;
  let k0=h.x*e.y-h.y*e.x;
  var t:f32;
  if (abs(k2)<0.00001) {
    t=-k0/k1;
  } else {
    let root=sqrt(max(k1*k1-4.0*k0*k2,0.0));
    t=(-k1-root)*0.5/k2;
    if (t<0.0||t>1.0) { t=(-k1+root)*0.5/k2; }
  }
  let across=e+g*t;
  return vec2<f32>(dot(h-f*t,across)/max(dot(across,across),0.000001),t);
}

@fragment
fn fs_road(input:RoadOutput)->@location(0) vec4<f32> {
  let behind=textureLoad(scene_color,vec2<i32>(input.position.xy),0);
  if (abs(behind.a-0.5)>0.1) { discard; }
  let z=textureLoad(scene_depth,vec2<i32>(input.position.xy),0);
  let view_depth=camera.forward.w/max(1.0-z,0.000001);
  let ndc=input.clip.xy/input.clip.w;
  let ray=camera.forward.xyz+camera.right.xyz*camera.right.w*ndc.x+camera.up.xyz*camera.up.w*ndc.y;
  let world=camera.origin.xyz+ray*view_depth;
  let at=road_coords(world.xy,input.left0.xy,input.right0.xy,input.right1.xy,input.left1.xy);
  if (any(at<vec2<f32>(0.0))||any(at>vec2<f32>(1.0))) { discard; }
  if (abs(world.z-mix(input.left0.z,input.left1.z,at.y))>ROAD_REACH) { discard; }
  let n=normalize(cross(dpdx(world),dpdy(world)));
  // The texture repeats along the road and spans its width once.
  let uv=vec2<f32>(mix(input.left0.w,input.left1.w,at.y),at.x);
  let source=textureSampleGrad(decal_map,decal_sampler,uv,dpdx(uv),dpdy(uv));
  let facing=clamp(abs(n.z)*3.0-0.5,0.0,1.0);
  let ends=mix(input.right0.w,input.right1.w,at.y);
  let view_fade=clamp((400.0-view_depth)/100.0,0.0,1.0);
  let alpha=source.a*input.tint.a*facing*ends*view_fade;
  if (alpha<=0.002) { discard; }
  // The road's relief: the same basis as other materials, with U along the road
  // and V across it. The ground below already holds its own sun, so the bump only
  // scales it by how much more or less sun the bent normal catches.
  let up=select(-n,n,n.z>=0.0);
  let along_raw=input.left1.xyz-input.left0.xyz;
  let across_raw=input.right0.xyz-input.left0.xyz;
  let tangent=normalize(along_raw-up*dot(up,along_raw));
  let bitangent=normalize(across_raw-up*dot(up,across_raw)-tangent*dot(tangent,across_raw));
  let bump=textureSampleGrad(road_normal,decal_sampler,uv,dpdx(uv),dpdy(uv));
  let xy=bump.rg*2.0-vec2<f32>(1.0);
  let bent=normalize(tangent*xy.x+bitangent*xy.y+up*sqrt(max(1.0-dot(xy,xy),0.0)));
  let sun_direction=normalize(light.sun.xyz);
  let luma=vec3<f32>(0.2126,0.7152,0.0722);
  let ambient=dot(light.ambient.rgb,luma);
  let sun=dot(light.sun_color.rgb,luma)*light.sun.w;
  let relief=(ambient+sun*max(dot(bent,sun_direction),0.0))
    /max(ambient+sun*max(dot(up,sun_direction),0.0),0.001);
  let incoming=behind.rgb/GROUND_ALBEDO;
  let to_camera=normalize(camera.origin.xyz-world);
  let shine=game_specular(bent,to_camera,sun_direction,road_specular.rgb,bump.a*road_specular.a)*incoming;
  return vec4<f32>(source.rgb*input.tint.rgb*incoming*clamp(relief,0.4,1.8)+shine,alpha);
}
