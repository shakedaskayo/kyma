# syntax=docker/dockerfile:1.7

# ----- Builder -----
FROM node:22-alpine AS builder
WORKDIR /app
RUN corepack enable

# Copy the source-of-truth markdown for the architecture include step.
COPY docs/architecture.md /app/docs/architecture.md
COPY docs/benchmarks.md /app/docs/benchmarks.md
COPY docs/images /app/docs/images

# Copy the docs site
COPY docs/site /app/docs/site

WORKDIR /app/docs/site
RUN pnpm install --frozen-lockfile=false
RUN pnpm run build

# ----- Runner -----
FROM node:22-alpine
RUN npm install -g serve@14
COPY --from=builder /app/docs/site/.vitepress/dist /site
ENV PORT=3000
EXPOSE 3000
CMD ["sh", "-c", "serve -l ${PORT} /site"]
