import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import { startCompileWorker } from './compile/port';
import { browserFontsEnvironment } from './fonts/environment';
import { alphaCopier, browserAlphaEnvironment, canMakeAlphaCopies } from './media/alpha';
import {
  startCompressWorker,
  startVideoWorker,
  workerCount,
  workerImageCompressor,
  workerVideoCompressor,
} from './media/compressor';
import { startDecodeWorker, workerPhotoDecoder } from './media/photos';
import { slidockPlayer } from './player/slidock-player';
import './styles.css';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App
        services={{
          startWorker: startCompileWorker,
          player: slidockPlayer,
          fonts: browserFontsEnvironment(),
          photos: () => workerPhotoDecoder(startDecodeWorker),
          images: () => workerImageCompressor(startCompressWorker, workerCount(navigator.hardwareConcurrency)),
          videos: () => workerVideoCompressor(startVideoWorker),
          ...(canMakeAlphaCopies() ? { alternates: () => alphaCopier(browserAlphaEnvironment()) } : {}),
        }}
      />
    </StrictMode>,
  );
}
