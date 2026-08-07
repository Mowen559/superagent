import { initShadowGit, commitShadowChange } from './gitnexus/dist/server/shadow-git.js';
import path from 'path';

async function test() {
  const repoPath = path.resolve('.');
  console.log('Testing shadow git on', repoPath);
  await initShadowGit(repoPath);
  console.log('Done initializing shadow git.');
}
test().catch(console.error);
