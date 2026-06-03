pkgname=edgemap
pkgver=0.4.3
pkgrel=1
pkgdesc="DualSense Edge UHID proxy — remap, combo, macro, profile auto-switching"
arch=('x86_64')
url="https://github.com/kurobac/edgemap"
license=('GPL3')
makedepends=('cargo')
optdepends=('libnotify: desktop notifications on profile switch')
install=edgemap.install

build() {
    cd "$startdir"
    cargo build --release --locked
}

package() {
    cd "$startdir"
    install -Dm755 target/release/dseuhid "$pkgdir/usr/bin/dseuhid"
    install -Dm755 target/release/edgemap "$pkgdir/usr/bin/edgemap"
    install -Dm644 dseuhid.service "$pkgdir/usr/lib/systemd/system/dseuhid.service"
    install -Dm644 edgemap.service "$pkgdir/usr/lib/systemd/user/edgemap.service"
}
