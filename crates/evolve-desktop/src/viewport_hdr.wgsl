// Shared linear HDR + bounded quarter-resolution bloom for the Model Viewer
// and Level Editor. This does not embed any original game shader binaries.
struct FsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_fullscreen(@builtin(vertex_index) idx:u32)->FsOut {
    let xy=array<vec2<f32>,3>(
        vec2<f32>(-1.0,-1.0), vec2<f32>(3.0,-1.0), vec2<f32>(-1.0,3.0));
    var result:FsOut;
    result.clip=vec4<f32>(xy[idx],0.0,1.0);
    result.uv=xy[idx]*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5,0.5);
    return result;
}
@group(0) @binding(0) var hdr_scene:texture_2d<f32>;
@group(0) @binding(1) var hdr_bloom:texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler:sampler;
@group(0) @binding(3) var<uniform> composite_params:vec4<f32>;
fn aces_fitted(radiance:vec3<f32>)->vec3<f32>{
    let v=max(radiance*composite_params.y,vec3<f32>(0.0));
    return clamp((v*(2.51*v+0.03))/(v*(2.43*v+0.59)+0.14),
        vec3<f32>(0.0),vec3<f32>(1.0));
}
@fragment
fn fs_composite(input:FsOut)->@location(0) vec4<f32> {
    // Unlike the quarter-res filter, this pass draws in the UI's scissored
    // viewport. Fragment coordinates must sample the scene at absolute
    // physical pixels, not stretch the whole window into the viewport.
    let size=vec2<f32>(textureDimensions(hdr_scene,0));
    let uv=clamp(input.clip.xy/size,vec2<f32>(0.0),vec2<f32>(1.0));
    let source=max(textureSampleLevel(hdr_scene,hdr_sampler,uv,0.0).rgb,vec3<f32>(0.0));
    if (composite_params.z > 0.5) {
        // Source RGB (unlit) is an explicit original-map color diagnostic;
        // no HDR film curve or bloom must interfere with source comparison.
        return vec4<f32>(source,1.0);
    }
    let bloom=textureSampleLevel(hdr_bloom,hdr_sampler,uv,0.0).rgb;
    let combined=source+bloom*max(composite_params.x,0.0);
    return vec4<f32>(aces_fitted(combined),1.0);
}
