// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// L1 probe: the byte offset Metal gives each field of a uniform block; the twin of l1_block_gl.c.
// Build: swiftc -o l1_block_metal l1_block_metal.swift
import Metal
let src = """
#include <metal_stdlib>
using namespace metal;
struct B { float4 a; float b; float2 c; float d; float4 e; float2 f; int n; float4 l[3]; float4 k[3]; float4x4 m; float z; };
vertex float4 vs(uint i [[vertex_id]]) { return float4(0); }
fragment float4 fs(constant B& u [[buffer(0)]]) { return float4(u.a.xyz+u.e.xyz+u.k[2].xyz, u.b+u.c.x+u.d+u.f.x+float(u.n)+u.l[2].x+u.m[3].x+u.z); }
"""
let d = MTLCreateSystemDefaultDevice()!
let lib = try! d.makeLibrary(source: src, options: nil)
let pd = MTLRenderPipelineDescriptor()
pd.vertexFunction = lib.makeFunction(name: "vs"); pd.fragmentFunction = lib.makeFunction(name: "fs")
pd.colorAttachments[0].pixelFormat = .bgra8Unorm
var refl: MTLRenderPipelineReflection?
_ = try! d.makeRenderPipelineState(descriptor: pd, options: [.argumentInfo, .bufferTypeInfo], reflection: &refl)
for arg in refl!.fragmentArguments! where arg.type == .buffer {
  for m in arg.bufferStructType!.members { print(m.name, m.offset, m.arrayType().map { "stride \($0.stride)" } ?? "") }
  print("size", arg.bufferDataSize)
}
