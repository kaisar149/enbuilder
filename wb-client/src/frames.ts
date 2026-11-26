// Binary frame layout (see wb-server/src/protocol.rs): "WBF1" | u32 LE json_len | json | zero padding to a multiple of 4 | payload.
import type { MeshData, MeshHeader } from './types';

export interface DecodedFrame {
  header: any;
  /** Byte offset of the payload inside `buffer`; always a multiple of 4. */
  payloadOffset: number;
  payloadBytes: number;
  buffer: ArrayBuffer;
}

export function decodeFrame(buffer: ArrayBuffer): DecodedFrame {
  const dv = new DataView(buffer);
  if (buffer.byteLength < 8 || dv.getUint8(0) !== 0x57 || dv.getUint8(1) !== 0x42 || dv.getUint8(2) !== 0x46 || dv.getUint8(3) !== 0x31) {
    throw new Error('not a WBF1 frame');
  }
  const jsonLen = dv.getUint32(4, true);
  const json = new TextDecoder().decode(new Uint8Array(buffer, 8, jsonLen));
  const payloadOffset = Math.ceil((8 + jsonLen) / 4) * 4;
  return { header: JSON.parse(json), payloadOffset, payloadBytes: buffer.byteLength - payloadOffset, buffer };
}

/** Copies `count` f32 values out of the payload (own buffer, so the frame can be garbage collected). */
export function f32Array(f: DecodedFrame, byteOffset: number, count: number): Float32Array {
  const start = f.payloadOffset + byteOffset;
  return new Float32Array(f.buffer.slice(start, start + count * 4));
}

export function u32Array(f: DecodedFrame, byteOffset: number, count: number): Uint32Array {
  const start = f.payloadOffset + byteOffset;
  return new Uint32Array(f.buffer.slice(start, start + count * 4));
}

/** Decodes the `get_mesh` answer using the layout table in its header (never hard-codes offsets). */
export function decodeMesh(header: MeshHeader, f: DecodedFrame): MeshData {
  const item = (name: string) => {
    const it = header.layout.find((l) => l.name === name);
    if (!it) throw new Error(`mesh layout has no '${name}'`);
    return it;
  };
  const c = item('centroid');
  const a = item('area');
  const ao = item('adj_offsets');
  const aj = item('adj');
  return {
    meshId: header.mesh_id,
    kind: header.kind,
    nCells: header.n_cells,
    centroid: f32Array(f, c.offset, c.count * 3),
    area: f32Array(f, a.offset, a.count),
    adjOffsets: u32Array(f, ao.offset, ao.count),
    adj: u32Array(f, aj.offset, aj.count),
  };
}
