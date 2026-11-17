# Backend B image for Render. Build from the repo root (the web app is built and embedded):
#   docker build -f backend-b/Dockerfile .

# 1. Web app (web/dist). pnpm 12 is a native binary; install.js links it because install scripts are off.
FROM node:26.13.0-alpine AS web
WORKDIR /repo
RUN npm install -g pnpm@12.21.0 --ignore-scripts && node "$(npm root -g)/pnpm/install.js"
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY web/package.json web/
COPY contract/package.json contract/
RUN pnpm install --frozen-lockfile --filter web
COPY design design
COPY web web
RUN pnpm -F web build

# 2. Go server with the web app embedded (internal/site)
FROM golang:1.27.4-alpine AS build
WORKDIR /src
COPY backend-b/go.mod backend-b/go.sum ./
RUN go mod download
COPY backend-b/ .
COPY --from=web /repo/web/dist internal/site/dist
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/server ./cmd/server \
 && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/migrate ./cmd/migrate

# 3. distroless static: no shell, no package manager, runs as uid 65532
FROM gcr.io/distroless/static-debian13:nonroot
COPY --from=build /out/server /out/migrate /app/
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/app/server"]
