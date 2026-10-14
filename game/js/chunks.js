// GLSL shared by terrain / water / sky / patched object materials so everything fogs and lights the same way.
export const fogUniforms = `
uniform vec3 uFogColor; uniform vec2 uFogRange; uniform float uFogDensity; uniform vec3 uSunDirView; uniform vec3 uSunColor;
`;
// view = view-space position of the fragment (camera at origin). Matches the game's linear fog (start/end as fraction of view distance)
// with a little forward scattering towards the sun.
export const fogFn = `
vec3 dfFog(vec3 col, vec3 viewPos, float fade) {
  float depth = length(viewPos);
  float f = smoothstep(uFogRange.x, uFogRange.y, depth) * uFogDensity;
  float sd = max(dot(normalize(viewPos), uSunDirView), 0.0);
  vec3 fc = uFogColor + uSunColor * (pow(sd, 5.0) * 0.18 + pow(sd, 40.0) * 0.25);
  col = mix(col, fc, clamp(f, 0.0, 1.0));
  return mix(fc, col, fade);          // freshly streamed-in chunks rise out of the fog
}
`;
