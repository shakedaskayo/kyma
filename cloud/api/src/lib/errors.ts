import { HTTPException } from 'hono/http-exception';

export class AppError extends HTTPException {
  constructor(public statusCode: number, public code: string, message: string) {
    super(statusCode as any, { message });
  }
}

export const badRequest    = (m: string, c = 'BAD_REQUEST')    => new AppError(400, c, m);
export const unauthorized  = (m = 'Unauthorized', c = 'UNAUTHORIZED')  => new AppError(401, c, m);
export const forbidden     = (m = 'Forbidden', c = 'FORBIDDEN')        => new AppError(403, c, m);
export const notFound      = (m = 'Not found', c = 'NOT_FOUND')        => new AppError(404, c, m);
export const conflict      = (m: string, c = 'CONFLICT')               => new AppError(409, c, m);
