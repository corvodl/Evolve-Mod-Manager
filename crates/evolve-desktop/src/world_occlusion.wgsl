// Screen-space ambient occlusion from scene depth, using each map's authored
// amount and radius. The game's own pass is not decoded, so the sampling pattern
// and bias are this renderer's own. It darkens the finished opaque scene, so
// sunlit surfaces darken a little too, where the game affects mainly ambient.
struct Camera { origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>, }
struct Occlusion { params:vec4<f32>, } // amount, radius, radius min, radius max
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<uniform> occlusion:Occlusion;
@group(0) @binding(2) var scene_depth:texture_depth_2d;
@group(0) @binding(3) var raw_occlusion:texture_2d<f32>;

struct Output { @builtin(position) position:vec4<f32>, @location(0) ndc:vec2<f32>, }

@vertex
fn vs_fullscreen(@builtin(vertex_index) i:u32)->Output {
  let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
  var out:Output;
  out.position=vec4<f32>(p[i],0.0,1.0);
  out.ndc=p[i];
  return out;
}

fn view_depth_at(pixel:vec2<i32>)->f32 {
  let size=vec2<i32>(textureDimensions(scene_depth));
  let z=textureLoad(scene_depth,clamp(pixel,vec2<i32>(0),size-vec2<i32>(1)),0);
  return camera.forward.w/max(1.0-z,0.000001);
}

@fragment
fn fs_occlusion(input:Output)->@location(0) vec4<f32> {
  let pixel=vec2<i32>(input.position.xy);
  let depth=view_depth_at(pixel);
  // Open sky and far terrain have nothing useful at this radius.
  if (depth>1200.0) { return vec4<f32>(1.0); }
  let ray=camera.forward.xyz+camera.right.xyz*camera.right.w*input.ndc.x+camera.up.xyz*camera.up.w*input.ndc.y;
  let world=camera.origin.xyz+ray*depth;
  var n=cross(dpdx(world),dpdy(world));
  if (dot(n,n)<0.0000001) { return vec4<f32>(1.0); }
  n=normalize(n);
  if (dot(n,camera.origin.xyz-world)<0.0) { n=-n; }
  let t=normalize(select(cross(n,vec3<f32>(0.0,0.0,1.0)),cross(n,vec3<f32>(1.0,0.0,0.0)),abs(n.z)>0.9));
  let b=cross(n,t);
  let radius=clamp(occlusion.params.y,occlusion.params.z,occlusion.params.w);
  // The viewport may be a part of the target, so scale by its measured size.
  let pixels_per_ndc=vec2<f32>(1.0/max(abs(dpdx(input.ndc.x)),0.000001),-1.0/max(abs(dpdy(input.ndc.y)),0.000001));
  // Interleaved gradient noise turns the fixed spiral per pixel; the blur removes it.
  let turn=fract(52.9829189*fract(dot(input.position.xy,vec2<f32>(0.06711056,0.00583715))))*6.2831853;
  var blocked=0.0;
  for (var i=0;i<16;i=i+1) {
    let f=(f32(i)+0.5)/16.0;
    let angle=f32(i)*2.3999632+turn;
    let spread=sqrt(f);
    let lift=sqrt(1.0-f);
    let direction=t*cos(angle)*spread+b*sin(angle)*spread+n*lift;
    let sample_world=world+n*0.02+direction*radius*(0.15+0.85*f);
    let delta=sample_world-camera.origin.xyz;
    let sample_depth=dot(delta,camera.forward.xyz);
    if (sample_depth<=camera.forward.w) { continue; }
    let sample_ndc=vec2<f32>(dot(delta,camera.right.xyz)/camera.right.w,dot(delta,camera.up.xyz)/camera.up.w)/sample_depth;
    let at=input.position.xy+(sample_ndc-input.ndc)*pixels_per_ndc;
    let surface=view_depth_at(vec2<i32>(at));
    let gap=sample_depth-surface;
    if (gap>0.01*radius) {
      // Geometry far in front is a silhouette, not something shading this point.
      blocked+=clamp(1.0-(gap-radius)/radius,0.0,1.0);
    }
  }
  let open=1.0-blocked/16.0;
  return vec4<f32>(pow(open,1.5));
}

// A depth-aware blur, then a multiply into the scene.
@fragment
fn fs_apply(input:Output)->@location(0) vec4<f32> {
  let pixel=vec2<i32>(input.position.xy);
  let size=vec2<i32>(textureDimensions(raw_occlusion));
  let depth=view_depth_at(pixel);
  var sum=0.0;
  var weight=0.0;
  for (var y=-2;y<=2;y=y+1) {
    for (var x=-2;x<=2;x=x+1) {
      let at=clamp(pixel+vec2<i32>(x,y),vec2<i32>(0),size-vec2<i32>(1));
      let similar=clamp(1.0-abs(view_depth_at(at)-depth)/max(depth*0.02,0.05),0.0,1.0);
      let w=0.05+similar;
      sum+=textureLoad(raw_occlusion,at,0).r*w;
      weight+=w;
    }
  }
  let shade=mix(1.0,sum/weight,clamp(occlusion.params.x,0.0,1.0));
  return vec4<f32>(vec3<f32>(shade),1.0);
}
