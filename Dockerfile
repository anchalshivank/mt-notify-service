# Stage 1: Build the application
FROM rust:latest AS builder

# Install the PostgreSQL client development library
RUN apt-get update && apt-get install -y libpq-dev

# Set the working directory inside the container
WORKDIR /usr/src/app

# Copy the Rust project files to the container
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the application in release mode
RUN cargo build --release

# Stage 2: Runtime environment
FROM ubuntu:latest

# Install the PostgreSQL runtime library
RUN apt-get update && apt-get install -y libpq5 && apt-get clean

# Set the working directory for the application
WORKDIR /app

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/notify-service .

# Expose the port your application listens on
EXPOSE 8080

# Set the default command to run the application
CMD ["./notify-service"]
