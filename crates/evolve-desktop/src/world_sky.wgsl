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
struct Celestial {
  bodies:array<vec4<f32>,4>,    // unit direction, angular radius in radians
  rotations:array<vec4<f32>,4>, // cos, sin of authored rotation, layer, enabled
}
@group(0) @binding(7) var<uniform> celestial:Celestial;
@group(0) @binding(8) var celestial_images:texture_2d_array<f32>;

// Planets sit behind the cloud dome and pick up the atmosphere in front of them.
fn over_celestial(direction:vec3<f32>,sky_color:vec3<f32>)->vec3<f32> {
  var color=sky_color;
  for (var i=0;i<4;i=i+1) {
    if (celestial.rotations[i].w<0.5) { continue; }
    let center=celestial.bodies[i].xyz;
    let facing=dot(direction,center);
    if (facing<=0.0) { continue; }
    var up=vec3<f32>(0.0,0.0,1.0);
    if (abs(center.z)>0.99) { up=vec3<f32>(0.0,1.0,0.0); }
    let right=normalize(cross(up,center));
    let top=cross(center,right);
    let scale=tan(celestial.bodies[i].w);
    var p=vec2<f32>(dot(direction,right),dot(direction,top))/(facing*scale);
    if (dot(p,p)>1.0) { continue; }
    let turn=celestial.rotations[i].xy;
    p=vec2<f32>(p.x*turn.x-p.y*turn.y,p.x*turn.y+p.y*turn.x);
    let sampled=textureSampleLevel(celestial_images,sky_sampler,p*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5),
      i32(celestial.rotations[i].z),0.0);
    color=mix(color,sampled.rgb*sunlight.sun_color.rgb*1.2+sky_color*0.25,sampled.a);
  }
  return color;
}

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
  let natural=(scattering*sun_rad*0.2 +
      sky.params[3].rgb*(0.10+0.22*max(direction.z,0.0)))*max(sun_dir.z+0.08,0.0);
  let disk=smoothstep(0.9993,0.99995,mu)*max(sun_dir.z+0.04,0.0);
  // Maps author these gradient colors for daytime as well as night, which is
  // where the warm horizon comes from. Blend shape and haze are calibrated.
  let height=clamp(direction.z/sky.params[9].w,0.0,1.0);
  let gradient=mix(sky.params[6].rgb*sky.params[6].w,
      sky.params[5].rgb*sky.params[5].w,height*(2.0-height));
  let falloff=(1.0-sky.params[8].w)*16.0;
  let sun_side=sky.params[8].rgb*pow(2.0,(mu-1.0)*falloff);
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
  linear=over_celestial(direction,linear);
  // Sky is written to the linear HDR render target; one final filmic
  // transform follows after bloom, never per-source double tonemapping.
  // Zero alpha marks open sky for the sun shaft pass.
  return vec4<f32>(linear,0.0);
}
