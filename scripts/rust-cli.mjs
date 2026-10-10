// Use Cargo metadata so tests also work with a shared or custom target folder.
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
export const cargoCommand = process.env.HWP_WORKBENCH_CARGO_WRAPPER || 'cargo';
let binary;

export function rustRpc(directory, method, args) {
  if (!binary) {
    const build = spawnSync(cargoCommand, ['build', '--locked', '-p', 'hwp-workbench'], {
      cwd: root, shell: false, stdio: ['ignore', 'ignore', 'inherit']
    });
    if (build.error) throw build.error;
    if (build.status !== 0) throw Error(`CLI build failed: ${build.status}`);
    const metadata = spawnSync(cargoCommand, ['metadata', '--no-deps', '--format-version', '1'], {
      cwd: root, shell: false, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']
    });
    if (metadata.error) throw metadata.error;
    if (metadata.status !== 0) throw Error(metadata.stderr);
    binary = path.join(JSON.parse(metadata.stdout).target_directory, 'debug', 'hwp-workbench');
  }
  const result = spawnSync(binary, ['rpc', '--root', directory], {
    cwd: root, shell: false, encoding: 'utf8', timeout: 30000, maxBuffer: 96 * 1024 * 1024,
    input: JSON.stringify({method, args})
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(result.stderr || `CLI exited ${result.status}`);
  return JSON.parse(result.stdout);
}
