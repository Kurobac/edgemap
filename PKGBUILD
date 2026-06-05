pkgname=edgemap
pkgver=0.7.3
pkgrel=1
pkgdesc="DualSense Edge UHID proxy — remap, combo, macro, profile auto-switching"
arch=('x86_64')
url="https://github.com/kurobac/edgemap"
license=('GPL3')
makedepends=('cargo')
optdepends=('libnotify: desktop notifications on profile switch'
            'python-pyqt6: GUI config editor (edgemap-gui)')
install=edgemap.install

build() {
    cd "$startdir"
    cargo build --release --locked
}

package() {
    cd "$startdir"
    install -Dm755 target/release/dseuhid "$pkgdir/usr/bin/dseuhid"
    install -Dm755 target/release/edgemap "$pkgdir/usr/bin/edgemap"
    install -Dm644 edgemap.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/edgemap.svg"
    install -Dm755 edgemap-gui-v6.py "$pkgdir/usr/bin/edgemap-gui"
    install -Dm644 edgemap.desktop "$pkgdir/usr/share/applications/edgemap.desktop"
    install -Dm644 dseuhid.service "$pkgdir/usr/lib/systemd/system/dseuhid.service"
    install -Dm644 edgemap.service "$pkgdir/usr/lib/systemd/user/edgemap.service"
}
