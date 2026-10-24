/**
 * 截图工具条图标（1:1 复刻参考图）。
 *
 * 设计约定：
 * - 统一 24×24 viewBox、`currentColor` 描边、圆头圆角，实际渲染 22px；
 * - 线宽 1.6（参考图在 48px 高的条里约 1.5px，等比还原）；
 * - 每个图标的 `scale` 按参考图实测的图形外接框标定（描边宽度已做反向补偿，
 *   放大图形不会让线条变粗）；
 * - 颜色由外层按钮控制（常规 #4c4c4c / 禁用 #aeaeae / 关闭 #fa5151 / 完成 #00c375）。
 */
import type { ReactNode, SVGProps } from 'react'

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, 'children' | 'scale'> {
  size?: number
}

function Icon({
  size = 22,
  scale = 1,
  children,
  ...rest
}: IconProps & { children: ReactNode; scale?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6 / scale}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      <g transform={`translate(12 12) scale(${scale}) translate(-12 -12)`}>{children}</g>
    </svg>
  )
}

/** 矩形 */
export const RectIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="3.6" y="3.6" width="16.8" height="16.8" rx="2.8" />
  </Icon>
)

/** 椭圆 */
export const EllipseIcon = (p: IconProps) => (
  <Icon scale={1.2} {...p}>
    <circle cx="12" cy="12" r="8.4" />
  </Icon>
)

/** 贴纸（笑脸） */
export const StickerIcon = (p: IconProps) => (
  <Icon scale={1.2} {...p}>
    <circle cx="12" cy="12" r="8.4" />
    <circle cx="9.3" cy="10.5" r="0.95" fill="currentColor" stroke="none" />
    <circle cx="14.7" cy="10.5" r="0.95" fill="currentColor" stroke="none" />
    <path d="M7.4 13.3h9.2a4.6 4.6 0 0 1-9.2 0Z" />
  </Icon>
)

/** 箭头（↗，L 形箭头头部） */
export const ArrowIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M5.5 18.6 18.4 5.7" />
    <path d="M11.9 5.7h6.5v6.5" />
  </Icon>
)

/** 画笔（斜置铅笔） */
export const PenIcon = (p: IconProps) => (
  <Icon scale={1.2} {...p}>
    <path d="M5.3 19.7l1.1-4.1L16 6.1a2.35 2.35 0 0 1 3.3 3.3L9.5 18.7l-4.2 1Z" />
    <path d="M15.1 7.5l3.2 3.2" />
  </Icon>
)

/** 马赛克（棋盘格） */
export const MosaicIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="3.6" y="3.6" width="16.8" height="16.8" rx="2.8" />
    <g fill="currentColor" stroke="none">
      {[6.95, 12, 17.05].map((x) =>
        [6.95, 12, 17.05].map((y) => (
          <rect key={`a${x}-${y}`} x={x - 0.825} y={y - 0.825} width="1.65" height="1.65" rx="0.4" />
        )),
      )}
      {[9.475, 14.525].map((x) =>
        [9.475, 14.525].map((y) => (
          <rect key={`b${x}-${y}`} x={x - 0.825} y={y - 0.825} width="1.65" height="1.65" rx="0.4" />
        )),
      )}
    </g>
  </Icon>
)

/** 文字（方框内的 T） */
/** 聚光灯：画面范围 + 中央亮区 + 光晕虚线圈 */
export const SpotlightIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="3.6" y="3.6" width="16.8" height="16.8" rx="3.2" />
    <circle cx="12" cy="12" r="3.4" fill="currentColor" stroke="none" />
    <circle cx="12" cy="12" r="6.7" strokeDasharray="2.2 2.7" />
  </Icon>
)

export const TextIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="3.6" y="3.6" width="16.8" height="16.8" rx="2.8" />
    <path d="M8.1 8.3h7.8" />
    <path d="M12 8.3v8.2" />
  </Icon>
)

/** 序号（圆环内的 1；数字用描边画，保持与其它图标同一线宽语言） */
export const SequenceIcon = (p: IconProps) => (
  <Icon scale={1.2} {...p}>
    <circle cx="12" cy="12" r="8.4" />
    <path d="M10.3 10.4l1.75-1.25v6.7" />
  </Icon>
)

/** 橡皮擦（倾斜的橡皮 + 擦除面分割线） */
export const EraserIcon = (p: IconProps) => (
  <Icon scale={1.2} {...p}>
    <g transform="rotate(-45 12 12)">
      <rect x="4.6" y="8.8" width="14.8" height="6.4" rx="1.9" />
      <path d="M9.2 8.8v6.4" />
    </g>
  </Icon>
)

/** 水印（边框 + 3×3 点阵，表达"平铺"） */
export const WatermarkIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="3.6" y="3.6" width="16.8" height="16.8" rx="2.8" />
    <g fill="currentColor" stroke="none">
      {[7.2, 12, 16.8].map((cx) =>
        [7.2, 12, 16.8].map((cy) => (
          <circle key={`w${cx}-${cy}`} cx={cx} cy={cy} r="1.05" />
        )),
      )}
    </g>
  </Icon>
)

/** 放大镜（圆镜 + 手柄 + 镜片内的加粗内容线，暗示"放大"） */
export const ZoomIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="10.4" cy="10.4" r="6.1" />
    <path d="M14.9 14.9 20.2 20.2" />
    <path d="M8 10.4h4.8" strokeWidth={2.4} />
  </Icon>
)

/** 智能擦除（倾斜的橡皮 + 右上角闪光，表达"内容感知"） */
export const SmartEraseIcon = (p: IconProps) => (
  <Icon scale={1.05} {...p}>
    <g transform="rotate(-45 12 12)">
      <rect x="4.2" y="9.6" width="11.6" height="5.2" rx="1.6" />
    </g>
    <path
      d="M18.4 4.6l.75 1.95 1.95.75-1.95.75-.75 1.95-.75-1.95-1.95-.75 1.95-.75Z"
      fill="currentColor"
      stroke="none"
    />
  </Icon>
)

/** 翻译（A / 文 两个叠放的方框） */
export const TranslateIcon = (p: IconProps) => (
  <Icon scale={1.22} {...p}>
    <rect x="11.3" y="2.7" width="9.5" height="9.5" rx="2" />
    <path d="M13.8 10.3l2.25-5.4 2.25 5.4" />
    <path d="M14.6 8.6h2.9" />
    <rect x="3.2" y="10.5" width="9.5" height="9.5" rx="2.2" fill="#fafafa" />
    <text
      x="7.95"
      y="16.6"
      textAnchor="middle"
      fontSize="8.1"
      fontFamily="'Microsoft YaHei','PingFang SC',sans-serif"
      fill="currentColor"
      stroke="none"
    >
      文
    </text>
  </Icon>
)

/** 文字识别（四角括号 + 文） */
export const OcrIcon = (p: IconProps) => (
  <Icon scale={1.26} {...p}>
    <path d="M10.2 4.3H6.4A2.1 2.1 0 0 0 4.3 6.4v3.8" />
    <path d="M13.8 4.3h3.8a2.1 2.1 0 0 1 2.1 2.1v3.8" />
    <path d="M10.2 19.7H6.4a2.1 2.1 0 0 1-2.1-2.1v-3.8" />
    <path d="M13.8 19.7h3.8a2.1 2.1 0 0 0 2.1-2.1v-3.8" />
    <text
      x="12"
      y="15.0"
      textAnchor="middle"
      fontSize="8.7"
      fontFamily="'Microsoft YaHei','PingFang SC',sans-serif"
      fill="currentColor"
      stroke="none"
    >
      文
    </text>
  </Icon>
)

/** 长截图（虚线框 + 上下双箭头） */
export const LongShotIcon = (p: IconProps) => (
  <Icon scale={1.28} {...p}>
    <path d="M5.9 7.2V6.4a1.3 1.3 0 0 1 1.3-1.3h9.6a1.3 1.3 0 0 1 1.3 1.3v.9" />
    <path d="M5.9 16.8v.8a1.3 1.3 0 0 0 1.3 1.3h9.6a1.3 1.3 0 0 0 1.3-1.3v-.9" />
    <path d="M5.9 8.6v2.7" strokeDasharray="2.3 2.1" />
    <path d="M5.9 13.3v2.7" strokeDasharray="2.3 2.1" />
    <path d="M18.1 8.6v2.7" strokeDasharray="2.3 2.1" />
    <path d="M18.1 13.3v2.7" strokeDasharray="2.3 2.1" />
    <path d="M12 7.7v8.6" />
    <path d="M9.5 10.1 12 7.6l2.5 2.5" />
    <path d="M9.5 13.9 12 16.4l2.5-2.5" />
  </Icon>
)

/** 撤销（向左的回转箭头） */
export const UndoIcon = (p: IconProps) => (
  <Icon scale={1.1} {...p}>
    <path d="M8.4 6.7 4.3 10.7l4.1 4" />
    <path d="M4.6 10.7h10.6a4.2 4.2 0 0 1 0 8.4H4" />
  </Icon>
)

/// 重做（向右的回转箭头，是撤销的**水平镜像**）。
///
/// x 轴镜像：`x' = 24 - x`。手写而不是给 UndoIcon 加 transform，
/// 是因为反转后的笔画端点要落到整数附近，缩放变换会让线条发虚。
export const RedoIcon = (p: IconProps) => (
  <Icon scale={1.1} {...p}>
    <path d="M15.6 6.7 19.7 10.7l-4.1 4" />
    <path d="M19.4 10.7H8.8a4.2 4.2 0 0 0 0 8.4H20" />
  </Icon>
)

/** 保存（托盘 + 向下箭头） */
export const SaveIcon = (p: IconProps) => (
  <Icon scale={1.28} {...p}>
    <path d="M12 3.6v10" />
    <path d="M8.5 10.4 12 13.9l3.5-3.5" />
    <path d="M4.1 12.2v5.3a2.5 2.5 0 0 0 2.5 2.5h10.8a2.5 2.5 0 0 0 2.5-2.5v-5.3" />
  </Icon>
)

/** 贴图（图钉） */
export const PinIcon = (p: IconProps) => (
  <Icon scale={1.15} {...p}>
    <path d="M10.5 5.2h3v2.2c0 1.2.7 2 1.8 2.9 1.8 1.6 2.5 3.2 2.5 5.2v1.1H6.2v-1.1c0-2 .7-3.6 2.5-5.2 1.1-.9 1.8-1.7 1.8-2.9V5.2Z" />
    <path d="M12 16.7v4.5" />
  </Icon>
)

/** 分享（右向弧线箭头） */
export const ShareIcon = (p: IconProps) => (
  <Icon scale={1.1} {...p}>
    <path d="M12.6 3.9 21.3 11.7 12.6 19.5" />
    <path d="M12.2 7.3C7.4 8.3 3.7 13.2 2.8 20.9c3.9-3.7 7.8-5.3 12.7-4" />
  </Icon>
)

/** 取消（红叉） */
export const CloseIcon = (p: IconProps) => (
  <Icon scale={1.17} {...p}>
    <path d="M6.5 6.5 17.5 17.5" />
    <path d="M17.5 6.5 6.5 17.5" />
  </Icon>
)

/** 完成（绿勾） */
export const CheckIcon = (p: IconProps) => (
  <Icon scale={1.4} {...p}>
    <path d="M5.2 12.6 9.8 17.1 18.8 7.5" />
  </Icon>
)

/** 取色器（吸管）：斜置滴管 + 管内液滴 + 右上取液点。
 *  与橡皮擦图标（EraserIcon）刻意区分：橡皮擦是"横握擦具体"，
 *  吸管是"斜握点液体"，且多一个取样点。 */
export const PickIcon = (p: IconProps) => (
  <Icon scale={1.12} {...p}>
    <path d="M13.6 4.2 19.8 10.4" />
    <path d="M17.2 3.1a2.05 2.05 0 0 1 2.9 2.9l-1.6 1.6-2.9-2.9 1.6-1.6Z" />
    <path d="M13.6 4.2 5.9 11.9a2 2 0 0 0-.5 2.1l.6 1.9-2.6 2.6a1.4 1.4 0 0 0 2 2l2.6-2.6 1.9.6a2 2 0 0 0 2.1-.5l7.7-7.7" />
  </Icon>
)

/** 二维码：三个定位角（回形）+ 数据区点阵。
 *  定位角用"外框 + 实心方点"画成回字形，数据区用实心小方块，
 *  与放大镜图标（圆 + 粗线）区分度更高。 */
export const QrIcon = (p: IconProps) => (
  <Icon scale={1.1} {...p}>
    <rect x="3.4" y="3.4" width="6.6" height="6.6" rx="1.5" />
    <rect x="14" y="3.4" width="6.6" height="6.6" rx="1.5" />
    <rect x="3.4" y="14" width="6.6" height="6.6" rx="1.5" />
    <rect x="6.1" y="6.1" width="1.9" height="1.9" rx="0.5" fill="currentColor" stroke="none" />
    <rect x="16.7" y="6.1" width="1.9" height="1.9" rx="0.5" fill="currentColor" stroke="none" />
    <rect x="6.1" y="16.7" width="1.9" height="1.9" rx="0.5" fill="currentColor" stroke="none" />
    <rect x="14.2" y="14.2" width="2.8" height="2.8" rx="0.6" fill="currentColor" stroke="none" />
    <rect x="18.2" y="18.2" width="2.4" height="2.4" rx="0.6" fill="currentColor" stroke="none" />
    <rect x="18.2" y="14.2" width="2.4" height="2.4" rx="0.6" fill="currentColor" stroke="none" />
    <rect x="14.2" y="18.2" width="2.4" height="2.4" rx="0.6" fill="currentColor" stroke="none" />
  </Icon>
)
