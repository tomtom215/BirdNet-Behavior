// The part of three.js the Soundscape terrain uses, and nothing else. esbuild follows these
// named re-exports and drops the rest of the library (the WebGPU renderer, the node material
// system, loaders, post-processing, ...), so what ships is one ES module of the WebGL
// renderer and the handful of classes below. static/terrain.js imports it with a dynamic
// import() the first time somebody opens the terrain.
//
// Add a name here only when static/terrain.js needs it, rebuild with build.sh, and the size
// budget (crates/birdnet-web/tests/vendored_three_is_pinned_and_small.rs) says what it cost.
export {
  AmbientLight,
  BufferAttribute,
  BufferGeometry,
  Color,
  DirectionalLight,
  Float32BufferAttribute,
  LineBasicMaterial,
  LineSegments,
  Mesh,
  MeshLambertMaterial,
  PerspectiveCamera,
  Raycaster,
  Scene,
  Vector2,
  Vector3,
  WebGLRenderer,
  REVISION,
} from "three";
