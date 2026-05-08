import type { SessionContext } from '../middleware/session.js';

export interface HonoEnv {
  Variables: {
    user: SessionContext;
  };
}
