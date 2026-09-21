// WebGPU flag values are fixed by the specification. TypeScript 6 exposes
// the flag types but no longer declares the browser's convenience globals.
export const GPU_FRAGMENT_STAGE: GPUShaderStageFlags = 0x2;
export const GPU_TEXTURE_BINDING: GPUTextureUsageFlags = 0x4;
export const GPU_RENDER_ATTACHMENT: GPUTextureUsageFlags = 0x10;
export const GPU_COPY_DST: GPUBufferUsageFlags = 0x8;
export const GPU_UNIFORM: GPUBufferUsageFlags = 0x40;
