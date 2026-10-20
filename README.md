# Emperor's Great Battle

Bản MVP game chiến thuật 1v1 chạy trên web. Rust/Axum quản lý phòng, WebSocket và mô phỏng trận đấu; trình duyệt dựng chiến trường cartoon 3D bằng Three.js với camera orthographic cố định.

## Tính năng có trong bản này

- Tên khách lưu cục bộ trong trình duyệt; không có đăng ký, đăng nhập, database tài khoản hay hồ sơ người chơi.
- Sảnh tìm kiếm danh sách phòng, tạo phòng, phòng có mật khẩu, kiểm tra trùng tên phòng.
- Trận 1v1 qua WebSocket; máy chủ giữ vàng, quân, HP, công trình, cứ điểm, lệnh và buff.
- Hai phe xanh/đỏ, màu được chọn ngẫu nhiên cho người mở phòng và người vào sau.
- Lâu đài hai bên, đồng cỏ top-down, sông nông có thể băng qua, hai cứ điểm chiếm đóng, giới hạn bản đồ/camera.
- Triển khai lính tiên phong, cung thủ, lính khiên, pháp sư, kỵ binh, kỵ tượng giáo, máy bắn đá, đại tướng quân và tối đa 5 mỏ vàng mỗi người.
- Cấp quân 1–5, nâng cấp mỏ, bốn lệnh điều quân, năm buff có tỷ lệ ngang nhau, sát thương lan/thiêu đốt/độc, tướng triệu hồi lính hoàng gia.
- Giao diện co giãn cho màn hình máy tính và điện thoại.

## Chạy máy cục bộ

Cần cài Rust toolchain (stable) và Cargo.

```bash
cargo run
```

Mở <http://localhost:10000>. Để thử 1v1, mở hai cửa sổ trình duyệt hoặc một cửa sổ ẩn danh. Cổng mặc định là `10000`; máy chủ cũng đọc biến môi trường `PORT`.

## Triển khai trên Render

1. Đưa thư mục này lên một repository GitHub của bạn.
2. Trong Render, chọn **New + → Blueprint** và kết nối repository; Render đọc cấu hình `render.yaml`. Hoặc tạo Web Service với runtime Docker.
3. Đảm bảo health-check path là `/healthz`, sau đó triển khai. WebSocket dùng cùng host (`wss://...`) khi trang được phục vụ qua HTTPS.
4. Mở URL Render được cấp và thử bằng hai trình duyệt/thiết bị khác nhau.

Mình không thể tạo service bên trong tài khoản Render của bạn khi chưa có quyền truy cập, nhưng repository này có Dockerfile và blueprint để Render build trực tiếp.

## Lưu ý triển khai / giới hạn MVP

- Trạng thái phòng và trận đấu được giữ trong bộ nhớ của tiến trình; khởi động lại hoặc deploy lại sẽ xóa các phòng. Không có tài khoản nào được lưu.
- Three.js được tải từ jsDelivr CDN, vì vậy trình duyệt người chơi cần truy cập CDN.
- Đây là MVP có gameplay online và AI mô phỏng đơn giản, chưa phải bản thương mại đã được stress-test. Cân bằng chỉ số/định tuyến mục tiêu và tối ưu nhiều phòng nên được kiểm thử thêm trước khi mở public rộng rãi.
- Bản đồ/đơn vị là mô hình low-poly 3D được tạo bằng hình học trong mã nguồn, không có asset bản quyền bên ngoài.

## Chỉ số quân mặc định (cấp 1)

| Quân | Giá | HP | ATK | DEF | SPEED | ATK SPEED | Tầm/đặc tính |
|---|---:|---:|---:|---:|---:|---:|---|
| Lính tiên phong | 5 | 54 | 4 | 4 | 3 | 5 | Cận chiến |
| Cung thủ | 7 | 34 | 8 | 2 | 2 | 7 | Tầm xa 105 |
| Lính khiên | 7 | 112 | 2 | 12 | 2 | 3 | Cận chiến, chống chịu |
| Pháp sư | 11 | 42 | 7 | 2 | 2 | 5 | AOE 20, thiêu đốt 4 giây |
| Kỵ binh | 10 | 70 | 8 | 5 | 5 | 5 | Lao thẳng, đánh quân trên đường lao |
| Kỵ tượng giáo | 12 | 98 | 9 | 7 | 5 | 6 | Tầm đánh 23 |
| Máy bắn đá | 14 | 60 | 15 | 5 | 1 | 2 | Tầm xa 145, AOE 26 |
| Đại tướng quân | 15 | 205 | 2 | 6 | 0 | 2 | Mỗi 3 giây gọi 2 lính hoàng gia |
| Lính hoàng gia | — | 69 | 5.5 | 5.5 | 4.5 | 6.5 | Chỉ do đại tướng quân triệu hồi |
| Mỏ vàng | 0 | 125 | — | — | — | — | +1 vàng/giây mỗi cấp, tối đa 5 mỏ và cấp 5 |

DEF giảm sát thương nhận vào; sát thương cơ bản tính bằng `max(ATK - 0.35 × DEF, 0.25 × ATK)`. ATK SPEED cao làm giảm thời gian giữa các đòn đánh (xấp xỉ `5 / ATK SPEED` giây). Cấp quân 2–5 tăng ATK/DEF/SPEED/ATK SPEED mỗi cấp 13%, HP mỗi cấp 18%; giá triển khai tăng 18% mỗi cấp. Mỏ mới luôn ở cấp 1 và cần dùng nút nâng mỏ để lên cấp.
