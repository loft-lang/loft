/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* L1 probe: the byte offset GL (std140, a 3.2 core context) gives each field of a uniform block.
 * Its twin is l1_block_metal.swift; API_GPU.md § Uniform blocks holds both outputs.
 * With `vec4 a` and `vec4 e` and `vec4 k[3]` written as vec3 the two stacks disagree from `b` on
 * (GL packs a float into a vec3's fourth word, Metal does not).
 * Build: cc -o l1_block_gl l1_block_gl.c -framework OpenGL */
#define GL_SILENCE_DEPRECATION
#include <OpenGL/OpenGL.h>
#include <OpenGL/gl3.h>
#include <stdio.h>
static GLuint sh(GLenum t,const char*s){GLuint h=glCreateShader(t);glShaderSource(h,1,&s,0);glCompileShader(h);GLint ok;glGetShaderiv(h,GL_COMPILE_STATUS,&ok);if(!ok){char l[999];glGetShaderInfoLog(h,999,0,l);printf("compile: %s\n",l);}return h;}
int main(void){
  CGLPixelFormatAttribute a[]={kCGLPFAOpenGLProfile,(CGLPixelFormatAttribute)kCGLOGLPVersion_3_2_Core,0};
  CGLPixelFormatObj pf;GLint n;CGLChoosePixelFormat(a,&pf,&n);CGLContextObj c;CGLCreateContext(pf,0,&c);CGLSetCurrentContext(c);
  const char*vs="#version 330 core\nvoid main(){gl_Position=vec4(0);}";
  const char*fs="#version 330 core\nlayout(std140) uniform B{vec4 a; float b; vec2 c; float d; vec4 e; vec2 f; int n; vec4 l[3]; vec4 k[3]; mat4 m; float z;};\nout vec4 o;void main(){o=vec4(a.xyz+e.xyz+k[2].xyz,b+c.x+d+f.x+float(n)+l[2].x+m[3].x+z);}";
  GLuint p=glCreateProgram();glAttachShader(p,sh(GL_VERTEX_SHADER,vs));glAttachShader(p,sh(GL_FRAGMENT_SHADER,fs));glLinkProgram(p);
  const char*names[]={"a","b","c","d","e","f","n","l","k","m","z"};GLuint idx[11];GLint off[11],str[11];
  glGetUniformIndices(p,11,names,idx);glGetActiveUniformsiv(p,11,idx,GL_UNIFORM_OFFSET,off);glGetActiveUniformsiv(p,11,idx,GL_UNIFORM_ARRAY_STRIDE,str);
  GLint size;glGetActiveUniformBlockiv(p,0,GL_UNIFORM_BLOCK_DATA_SIZE,&size);
  for(int i=0;i<11;i++)printf("%s %d stride %d\n",names[i],off[i],str[i]);printf("size %d\n",size);return 0;}
