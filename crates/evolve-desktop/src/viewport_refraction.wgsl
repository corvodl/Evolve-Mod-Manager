// Source-backed screen-space refraction building block.
// Reads an independent opaque-scene snapshot; never reads and writes the
// same wgpu texture. Shader is not enabled for unverified effect materials.
// Future pass will render ONLY Glass/Water or author-marked REFRACTION masks.
struct Params { strength:vec4<f32>, }
@group(0) @binding(0) var scene_color:texture_2d<f32>;
@group(0) @binding(1) var authored_normal:texture_2d<f32>;
@group(0) @binding(2) var coverage_mask:texture_2d<f32>;
@group(0) @binding(3) var source_depth:texture_depth_2d;
@group(0) @binding(4) var source_sampler:sampler;
@group(0) @binding(5) var<uniform> params:Params;
struct Out {
 @builtin(position) position:vec4<f32>,
 @location(0) uv:vec2<f32>,
}
@vertex
fn vs_refract(@builtin(vertex_index) index:u32)->Out {
 let tri=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
 var out:Out;
 out.position=vec4<f32>(tri[index],0.0,1.0);
 out.uv=tri[index]*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5,0.5);
 return out;
}
@fragment
fn fs_refract(input:Out)->@location(0) vec4<f32> {
 let size=vec2<f32>(textureDimensions(scene_color,0));
 let uv=clamp(input.position.xy/size,vec2<f32>(0.0),vec2<f32>(1.0));
 let coverage=textureSampleLevel(coverage_mask,source_sampler,uv,0.0);
 let scene=textureSampleLevel(scene_color,source_sampler,uv,0.0);
 if (coverage.a<=0.0001) {return vec4<f32>(scene.rgb,1.0);}
 let xy=textureSampleLevel(authored_normal,source_sampler,uv,0.0).rg*2.0-vec2<f32>(1.0);
 let pixel_offset=clamp(params.strength.x,0.0,16.0)*xy*coverage.a/max(size,vec2<f32>(1.0));
 let shifted=clamp(uv+pixel_offset,vec2<f32>(0.001),vec2<f32>(0.999));
 let pixel=vec2<i32>(clamp(shifted*size,vec2<f32>(0.0),size-vec2<f32>(1.0)));
 let depth_original=textureLoad(source_depth,vec2<i32>(input.position.xy),0);
 let depth_shifted=textureLoad(source_depth,pixel,0);
 // Limit color leaking across nearby silhouettes. Depth precision is
 // nonlinear; a small threshold is an editor heuristic, not native depth.
 let permitted=1.0-smoothstep(0.002,0.03,abs(depth_shifted-depth_original));
 let offset=textureSampleLevel(scene_color,source_sampler,shifted,0.0);
 return vec4<f32>(mix(scene.rgb,offset.rgb,coverage.a*permitted),1.0);
}
