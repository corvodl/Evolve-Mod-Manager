// Map-owned sky background, before terrain/world depth is drawn.
// CryEngine static Sky materials are three original packed textures (_12,
// _34, _5). Dynamic sky uses source TimeOfDay scattering coefficients;
// the integration model is a physically motivated preview, NOT Evolve shader
// bytecode or a validated native scattering LUT.

struct Camera {
  origin:vec4<f32>, forward:vec4<f32>, right:vec4<f32>, up:vec4<f32>,
}
struct SourceAtmosphere { params:array<vec4<f32>,10>, }
struct TimeOfDayLight {sun:vec4<f32>,ambient:vec4<f32>,sun_color:vec4<f32>,}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<uniform> sky:SourceAtmosphere;
@group(0) @binding(2) var sky_sampler:sampler;
@group(0) @binding(3) var sky_12:texture_2d<f32>;
@group(0) @binding(4) var sky_34:texture_2d<f32>;
@group(0) @binding(5) var sky_5:texture_2d<f32>;
@group(0) @binding(6) var<uniform> sunlight:TimeOfDayLight;

struct SkyVertex { @builtin(position) position:vec4<f32>, @location(0) ndc:vec2<f32>, }
@vertex
fn vs_sky(@builtin(vertex_index) i:u32)->SkyVertex {
  let p=array<vec2<f32>,3>(
    vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0)
  );
  var result:SkyVertex;
  result.position=vec4<f32>(p[i],1.0,1.0);
  result.ndc=p[i];
  return result;
}

fn aces_tone(linear:vec3<f32>)->vec3<f32> {
  let exposure=sky.params[7].x;
  let white=sky.params[7].y;
  let saturation=sky.params[7].z;
  let scaled=max(linear*exposure,vec3<f32>(0.0));
  let balance=scaled*(1.0+0.05*white);
  let mapped=clamp((balance*(2.51*balance+0.03))/
    (balance*(2.43*balance+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
  let luma=dot(mapped,vec3<f32>(0.2126,0.7152,0.0722));
  return clamp(mix(vec3<f32>(luma),mapped,saturation),vec3<f32>(0.0),vec3<f32>(1.0));
}

fn static_sky(direction:vec3<f32>)->vec3<f32> {
  let angle=sky.params[2].w;
  let rotated=vec3<f32>(
    direction.x*cos(angle)-direction.y*sin(angle),
    direction.x*sin(angle)+direction.y*cos(angle),
    direction.z
  );
  let a=abs(rotated);
  let stretch=max(sky.params[4].w,0.1);
  if (rotated.z>=max(a.x,a.y)) {
    let uv=clamp(vec2<f32>(0.5+0.5*rotated.x/max(a.z,0.0001),
      0.5-0.5*rotated.y/max(a.z,0.0001)),vec2<f32>(0.001),vec2<f32>(0.999));
    return textureSampleLevel(sky_5,sky_sampler,uv,0.0).rgb;
  }
  var face=0u;
  var u=0.5;
  var v=0.5;
  if (a.x>=a.y) {
    if (rotated.x>0.0) {
      face=0u;
      u=0.5-0.5*rotated.y/a.x;
    } else {
      face=2u;
      u=0.5+0.5*rotated.y/a.x;
    }
    v=0.5-0.5*rotated.z/a.x*stretch;
  } else {
    if (rotated.y>0.0) {
      face=1u;
      u=0.5+0.5*rotated.x/a.y;
    } else {
      face=3u;
      u=0.5-0.5*rotated.x/a.y;
    }
    v=0.5-0.5*rotated.z/a.y*stretch;
  }
  let uv=vec2<f32>(
    (clamp(u,0.001,0.999)+f32(face%2u))*0.5,
    clamp(v,0.001,0.999)
  );
  if (face<2u) { return textureSampleLevel(sky_12,sky_sampler,uv,0.0).rgb; }
  return textureSampleLevel(sky_34,sky_sampler,uv,0.0).rgb;
}

// Analytic single-scattering preview, with Rayleigh wavelength dependence,
// Henyey-Greenstein aerosol phase and daylight/twilight extinction. Original
// sky coefficients from TimeOfDay control it but it omits Evolve's exact
// multiple scattering, terrain shadows and runtime LUT.
fn dynamic_sky(direction:vec3<f32>)->vec3<f32> {
  let sun_dir=normalize(sunlight.sun.xyz);
  let mu=clamp(dot(direction,sun_dir),-0.9999,0.9999);
  let a=sky.params[1];
  let wavelengths=sky.params[2].xyz;
  let beta_ray=pow(vec3<f32>(550.0)/wavelengths,vec3<f32>(4.0))*
      max(0.0,a.x)*0.035;
  let beta_mie=max(0.0,a.y)*0.007;
  let cos_zenith=clamp(direction.z,-0.2,1.0);
  let air_mass=1.0/(max(cos_zenith,0.0)+0.08);
  let extinction=exp(-(beta_ray+vec3<f32>(beta_mie))*air_mass);
  let ray_phase=3.0/(16.0*3.14159265)*(1.0+mu*mu);
  let g=clamp(-a.z,-0.999,0.999);
  let denom=max(0.001,1.0+g*g-2.0*g*mu);
  let mie_phase=(1.0-g*g)/(4.0*3.14159265*pow(denom,1.5));
  let scattering=(beta_ray*ray_phase+vec3<f32>(beta_mie*mie_phase))*
      (vec3<f32>(1.0)-extinction)/
      max(beta_ray+vec3<f32>(beta_mie),vec3<f32>(0.001));
  let sun_rad=sky.params[0].rgb*(sky.params[0].a*sunlight.sun.w);
  let natural=(scattering*sun_rad*3.0 +
      sky.params[3].rgb*(0.10+0.22*max(direction.z,0.0)))*max(sun_dir.z+0.08,0.0);
  let disk=smoothstep(0.9993,0.99995,mu)*max(sun_dir.z+0.04,0.0);
  // Maps author these gradient colors for daytime as well as night, which is
  // where the warm horizon comes from. Blend shape and haze are calibrated.
  let height=clamp(direction.z/sky.params[9].w,0.0,1.0);
  let gradient=mix(sky.params[6].rgb*sky.params[6].w,
      sky.params[5].rgb*sky.params[5].w,height*(2.0-height));
  let sharpness=1.0/max(1.0-sky.params[8].w,0.001);
  let sun_side=sky.params[8].rgb*pow(2.0,(mu-1.0)*sharpness);
  let horizon=exp(-max(direction.z,0.0)*7.0);
  return max(natural+sun_rad*disk*0.7+gradient+sun_side*(0.35+0.65*horizon)+
      sky.params[9].rgb*horizon,vec3<f32>(0.0));
}

@fragment
fn fs_sky(input:SkyVertex)->@location(0) vec4<f32> {
  let direction=normalize(camera.forward.xyz
     +input.ndc.x*camera.right.xyz*camera.right.w
     +input.ndc.y*camera.up.xyz*camera.up.w);
  var linear=dynamic_sky(direction);
  if (sky.params[1].w>0.5) {
    linear=static_sky(direction);
  }
  // Sky is written to the linear HDR render target; one final filmic
  // transform follows after bloom, never per-source double tonemapping.
  return vec4<f32>(linear,1.0);
}
