import type { WorkerPort } from './compile/port';
import type { FontsEnvironment } from './fonts/controller';
import type { AlphaCopier } from './media/alpha';
import type { ImageCompressor, VideoCompressor } from './media/compressor';
import type { PhotoDecoder } from './media/photos';
import type { PlayerFactory } from './player/types';

/**
 * What the app runs on: the compile worker, the player, the browser the fonts come from, the decoder of HEIC
 * photos (none: they stay omitted), the encoders of a download's smaller pictures and videos (none: they stay as
 * they are) and the copier of its videos only some browsers play (none: they have no copy).
 * Tests pass fakes.
 */
export interface AppServices {
  startWorker(): WorkerPort;
  player: PlayerFactory;
  fonts: FontsEnvironment;
  photos?: () => PhotoDecoder;
  images?: () => ImageCompressor;
  videos?: () => VideoCompressor;
  alternates?: () => AlphaCopier;
}
