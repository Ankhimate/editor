import { cp, mkdir, rm, writeFile } from 'node:fs/promises';

const output = 'dist/api';
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
await cp('target/doc', output, { recursive: true });
await writeFile(
  `${output}/index.html`,
  `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Ankhimate Rust API</title><meta http-equiv="refresh" content="0; url=ankhimate_core/"><p>Open the <a href="ankhimate_core/">Ankhimate core API reference</a>.</p>`,
);
