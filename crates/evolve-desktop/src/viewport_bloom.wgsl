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
@group(0) @binding(0) var source_texture:texture_2d<f32>;
@group(0) @binding(1) var source_sampler:sampler;
@group(0) @binding(2) var<uniform> filter_params:vec4<f32>;
@group(0) @binding(3) var scene_log_luminance:texture_2d<f32>;

// The game has no bright pass: it blurs the whole scene, downscaled.
@fragment
fn fs_downscale(input:FsOut)->@location(0) vec4<f32> {
    let half_texel=filter_params.xy;
    var color=vec3<f32>(0.0);
    for (var i=0;i<4;i=i+1) {
        let side=vec2<f32>(f32(i&1)*2.0-1.0,f32(i>>1u)*2.0-1.0);
        color+=max(textureSampleLevel(source_texture,source_sampler,input.uv+side*half_texel,0.0).rgb,vec3<f32>(0.0));
    }
    return vec4<f32>(color*0.25,1.0);
}
// HDRBloomGaussianPS: 15 taps, each limited to 200 times a luminance texture. That
// texture is unnamed in the compiled shader; it is taken to be the scene's average.
@fragment
fn fs_blur(input:FsOut)->@location(0) vec4<f32> {
    let weights=array<f32,15>(0.000584,0.003113,0.011675,0.032689,0.070826,0.121417,0.166948,
        0.185497,0.166948,0.121417,0.070826,0.032689,0.011675,0.003113,0.000584);
    let limit=vec3<f32>(exp(textureLoad(scene_log_luminance,vec2<i32>(0,0),0).r)*200.0);
    var color=vec3<f32>(0.0);
    for (var i=0;i<15;i=i+1) {
        let uv=input.uv+filter_params.xy*(f32(i)-7.0);
        color+=min(textureSampleLevel(source_texture,source_sampler,uv,0.0).rgb*weights[i],limit);
    }
    return vec4<f32>(color,1.0);
}
// One blurred level's share of the final bloom, added by the blend state.
@fragment
fn fs_combine(input:FsOut)->@location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(source_texture,source_sampler,input.uv,0.0).rgb*filter_params.z,1.0);
}
// Mean log luminance of the scene, for authored eye adaptation.
@fragment
fn fs_luminance(input:FsOut)->@location(0) vec4<f32> {
    var sum=0.0;
    for (var y=0;y<16;y=y+1) {
        for (var x=0;x<16;x=x+1) {
            let uv=(vec2<f32>(f32(x),f32(y))+vec2<f32>(0.5))/16.0;
            let color=textureSampleLevel(source_texture,source_sampler,uv,0.0).rgb;
            sum+=log(dot(color,vec3<f32>(0.2126,0.7152,0.0722))+0.000001);
        }
    }
    return vec4<f32>(sum/256.0,0.0,0.0,1.0);
}
// Light from open sky, smeared toward the sun. filter_params: sun uv, strength.
@fragment
fn fs_shafts(input:FsOut)->@location(0) vec4<f32> {
    if (filter_params.z<=0.0) { return vec4<f32>(0.0,0.0,0.0,1.0); }
    let to_sun=filter_params.xy-input.uv;
    var sum=vec3<f32>(0.0);
    var weight=1.0;
    var total=0.0;
    for (var i=0;i<48;i=i+1) {
        let uv=input.uv+to_sun*(f32(i)/48.0);
        if (all(uv>=vec2<f32>(0.0)) && all(uv<=vec2<f32>(1.0))) {
            let sampled=textureSampleLevel(source_texture,source_sampler,uv,0.0);
            sum+=min(sampled.rgb,vec3<f32>(8.0))*(1.0-clamp(sampled.a*2.0,0.0,1.0))*weight;
        }
        total+=weight;
        weight*=0.97;
    }
    return vec4<f32>(sum/total,1.0);
}
