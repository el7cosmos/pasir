# Pasir

**PHP Application Server In Rust**

Pasir is an Indonesian word for "sands" (pronounced "PA-seer").

It is a high-performance PHP application server written in Rust that provides a solid foundation for PHP applications by
embedding PHP execution directly into a modern HTTP server. Built with a custom SAPI and C bindings through ext-php-rs,
Pasir offers a fast and efficient alternative to traditional PHP-FPM setups while maintaining compatibility with
existing PHP applications.

> ⚠️ **Development Status**: Pasir is currently in active development and not yet production-ready. While functional, it
> should be considered experimental for production use cases.

## ✨ Features

- **🚀 High Performance**: Built on Rust's async ecosystem with Hyper and Tokio for optimal performance
- **⚙️ Minimal Configuration**: Works out of the box without configuration, but offers intuitive TOML-based routing
  that's friendlier than .htaccess, nginx.conf, or Caddyfile when customization is needed
- **🔧 Embedded PHP**: Custom SAPI integration with PHP 8.1+ (ZTS required)
- **📡 Modern HTTP**: Full HTTP/1.1 and HTTP/2 support with automatic protocol detection
- **🎯 Flexible Routing**: Regex-based URL pattern matching with configurable handlers
- **📁 Static File Serving**: Built-in static file server with gzip compression support
- **🔄 Non-Persistent Execution**: Similar behavior to PHP-FPM for application compatibility
- **🛡️ Graceful Shutdown**: SIGINT handling with configurable timeout
- **📊 Request Tracing**: Built-in HTTP request tracing and logging
- **🐳 Docker Ready**: Containerized deployment support
- **⚡ Zero Downtime**: Hot-swappable connections during graceful shutdown

## 🛠️ Installation

### For End Users

#### Prerequisites

- **PHP**: Version 8.1+ compiled with `--disable-zend-signals --enable-embed --enable-zts`

#### Using Homebrew

```bash
brew install el7cosmos/pasir/pasir
```

#### Using Pre-built Binaries

```bash
# Download the latest release from GitHub
# Extract and place the binary in your PATH
```

#### Using Docker Images

Pre-built Docker images are available on Docker Hub at [el7cosmos/pasir](https://hub.docker.com/r/el7cosmos/pasir).

```bash
# Pull the latest image
docker pull el7cosmos/pasir

# Run with your PHP application
docker run -p 8080:8080 -v /path/to/your/app:/app el7cosmos/pasir

# Run with custom port and address
docker run -p 3000:3000 -v /path/to/your/app:/app el7cosmos/pasir --address 0.0.0.0 --port 3000
```

### Building from Source

#### Prerequisites

- **Rust**: Version 1.x (2024 edition)
- **PHP**: Version 8.1+ compiled with `--disable-zend-signals --enable-embed --enable-zts`
- **System Dependencies**: `libclang-dev` for ext-php-rs compilation

#### Build Steps

```bash
# Clone the repository
git clone <repository-url>
cd pasir

# Build the project
cargo build --release

# Run with default configuration
cargo run --release
```

### Docker Installation

```bash
# Build the Docker image
docker build -t pasir .

# Run with Docker
docker run -p 8080:8080 -v $(pwd):/app pasir

# Using docker-bake
docker buildx bake
```

## 🚀 Usage

### Basic Usage

```bash
# Run with required port (required parameter)
pasir --port 8080

# Custom address and port with document root
pasir --address 0.0.0.0 --port 3000 /path/to/your/webroot

# Enable verbose logging
pasir --port 8080 --verbose

# Quiet logging
pasir --port 8080 --quiet
```

### Command Line Options

```bash
PHP Application Server In Rust

Usage: pasir [OPTIONS] --port <PORT> [ROOT]

Arguments:
  [ROOT]  [default: .]

Options:
  -a, --address <ADDRESS>   [env: PASIR_ADDRESS=] [default: 127.0.0.1]
  -p, --port <PORT>         [env: PASIR_PORT=]
  -d, --define <foo[=bar]>  Define INI entry foo with value 'bar'
  -i, --info                PHP information
  -m, --modules             Show compiled in modules
  -v, --verbose...          Increase logging verbosity
  -q, --quiet...            Decrease logging verbosity
  -h, --help                Print help
  -V, --version             Print version
```

### Configuration

Pasir uses a `pasir.toml` configuration file in your document root to define routing rules:

```toml
# Serve PHP files
[[routes]]
match.uri = '(/(index|update)\.php)|(/core/[^/]*\.php$)'
serve = "php"

# Serve static files
[[routes]]
match.uri = 'favicon.ico'
serve = "static"

# Block sensitive files (optional)
[[routes]]
match.uri = [
    "*/composer.json",
    "*/composer.lock",
]
action.status = 404
serve = "default"

# Custom headers for assets (optional)
[[routes]]
match.uri = [
    "*.css",
    "*.js",
]
action.response_headers.append = [
    { "Cache-Control" = "public, max-age=3600" }
]

[[routes]]
match = { }
action.response_headers.remove = [
    "X-Generator"
]
```

#### Configuration Options

- **`match.uri`**: Regex pattern(s) for URL matching
- **`serve`**: Handler type (`"php"`, `"static"`, or `"default"`). When specified, directly serves the request without
  processing other route matches further
- **`action.status`**: HTTP status code for direct responses
- **`action.response_headers`**: Header manipulation (insert, append, remove)

### Docker Deployment

```bash
# Run with custom configuration
docker run -d \
  --name pasir-server \
  -p 8080:8080 \
  -v /path/to/your/app:/app \
  -v /path/to/pasir.toml:/app/pasir.toml \
  pasir

# With environment variables
docker run -d \
  --name pasir-server \
  -p 3000:3000 \
  -e PASIR_ADDRESS=0.0.0.0 \
  -e PASIR_PORT=3000 \
  -v /path/to/your/app:/app \
  pasir --address $PASIR_ADDRESS --port $PASIR_PORT
```

### Example PHP Application

Create a simple `index.php` in your document root:

```php
<?php
echo "Hello from Pasir!\n";
echo "Current time: " . date('Y-m-d H:i:s') . "\n";
echo "Server: " . $_SERVER['HTTP_HOST'] . "\n";
?>
```

With the basic configuration above, this will be served at `http://localhost:8080/index.php`.

## 🏗️ Architecture

Pasir combines several key technologies:

- **Rust + Tokio**: Async runtime for high-concurrency request handling
- **Hyper**: Modern HTTP server implementation
- **ext-php-rs**: PHP SAPI integration for embedded execution
- **Tower**: Middleware and service abstractions
- **Regex**: Flexible URL pattern matching

The server processes each request through:

1. **Router Service**: Matches URLs against configured patterns
2. **PHP Service**: Executes PHP scripts in embedded environment
3. **Static Service**: Serves static files with optimizations
4. **Response Pipeline**: Applies headers and transformations

## 🤝 Contributing

We welcome contributions! Please see our development guidelines:

1. **Code Style**: Follow the project's Rustfmt configuration (2-space indentation)
2. **Testing**: Add tests for new functionality using `cargo test`
3. **Documentation**: `todo!`
4. **Security**: Review any unsafe blocks and PHP integration code carefully

### Development Setup

```bash
# Clone and setup
git clone <repository-url>
cd pasir

# Install development dependencies
cargo build

# Run tests
cargo test

# Format code
cargo fmt

# Lint code
cargo clippy
```

### Docker Development

```bash
# Build development image
docker build -t pasir-dev .

# Run with hot reload (bind mount for development)
docker run -it --rm \
  -p 8080:8080 \
  -v $(pwd):/workspace \
  -w /workspace \
  pasir-dev
```

## 📝 License

This project is licensed under the terms specified in the LICENSE file.

## 🔗 Related Projects

- [PHP-FPM](https://www.php.net/manual/en/install.fpm.php) - Traditional FastCGI Process Manager
- [FrankenPHP](https://frankenphp.dev/) - Go-based PHP application server
- [RoadRunner](https://roadrunner.dev/) - High-performance PHP application server
- [NGINX Unit](https://unit.nginx.org/) - Polyglot web application server

## 🆘 Support

- **Issues**: Report bugs and feature requests via GitHub Issues
- **Documentation**: `todo!`
- **Discussions**: `todo!`

---

**Pasir** - Bringing modern performance to PHP applications through Rust's reliability and speed.