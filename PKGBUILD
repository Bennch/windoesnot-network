pkgname=windoesnot-network
pkgver=0.1.0
pkgrel=1
pkgdesc='WinDoesNot Network, adapted from Adwaita Network'
arch=(x86_64)
url='https://github.com/Bennch/windoesnot-network'
license=('GPL-3.0-or-later')
depends=(gtk4 libadwaita networkmanager libsecret polkit)
makedepends=(rust pkgconf)
options=('!lto' '!debug')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')
prepare() {
 cd "$srcdir/$pkgname-$pkgver"
 cargo fetch --locked
}
build() {
 cd "$srcdir/$pkgname-$pkgver"
 cargo build --release --locked
}
check() {
 cd "$srcdir/$pkgname-$pkgver"
 cargo test --release --locked desktop::tests
}
package() {
 cd "$srcdir/$pkgname-$pkgver"
 install -Dm755 target/release/$pkgname "$pkgdir/usr/bin/$pkgname"
 install -Dm644 packaging/me.windoesnot.Network.desktop "$pkgdir/usr/share/applications/me.windoesnot.Network.desktop"
 install -Dm644 packaging/me.windoesnot.Network.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/me.windoesnot.Network.svg"
 install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
