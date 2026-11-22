import { existsSync } from 'node:fs';
import { delimiter, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const env = { ...process.env };
const cargo = resolve('.toolchain/cargo');
if (existsSync(resolve(cargo, 'bin/cargo.exe'))) {
  env.CARGO_HOME = cargo;
  env.RUSTUP_HOME = resolve('.toolchain/rustup');
  const pathKey = Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'PATH';
  env[pathKey] = [resolve(cargo, 'bin'), env[pathKey]].filter(Boolean).join(delimiter);
}

const cli = spawn(process.execPath, ['node_modules/@tauri-apps/cli/tauri.js', ...process.argv.slice(2)], { env, stdio: 'inherit' });
cli.on('exit', (code) => { process.exitCode = code ?? 1; });
