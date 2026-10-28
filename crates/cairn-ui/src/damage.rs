//! 脏区记录。简单版本：最多保留几个矩形，相交或贴近的合并，超过上限就合成一个外接矩形。

use crate::{DeviceRect, Rect};

const MAX_RECTS: usize = 8;

#[derive(Debug, Default)]
pub(crate) struct Damage {
    rects: Vec<Rect>,
    full: bool,
}

/// 一帧要重画的内容。
#[derive(Debug, Default)]
pub(crate) struct DamageSet {
    pub full: bool,
    pub rects: Vec<Rect>,
}

impl Damage {
    pub fn add(&mut self, r: Rect) {
        if self.full || r.is_empty() {
            return;
        }
        let mut r = r;
        // 与已有矩形相交（含贴边）就合并，合并后可能又与别的相交，循环直到稳定。
        loop {
            let slack = r.inflate(2.0, 2.0);
            match self.rects.iter().position(|o| o.intersects(&slack)) {
                Some(i) => r = r.union(&self.rects.swap_remove(i)),
                None => break,
            }
        }
        self.rects.push(r);
        if self.rects.len() > MAX_RECTS {
            let all = self.rects.iter().fold(Rect::ZERO, |acc, r| acc.union(r));
            self.rects.clear();
            self.rects.push(all);
        }
    }

    pub fn add_full(&mut self) {
        self.full = true;
        self.rects.clear();
    }

    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }

    pub fn take(&mut self) -> DamageSet {
        DamageSet {
            full: std::mem::take(&mut self.full),
            rects: std::mem::take(&mut self.rects),
        }
    }
}

/// 逻辑脏区 → 物理像素区域：外扩 `margin`（抗锯齿边缘、焦点框），向外取整，合并重叠的。
pub(crate) fn to_device(rects: &[Rect], scale: f32, margin: f32, bounds: DeviceRect) -> Vec<DeviceRect> {
    let mut out: Vec<DeviceRect> = Vec::new();
    for r in rects {
        let Some(mut d) = DeviceRect::cover(&r.inflate(margin, margin), scale).intersection(&bounds) else {
            continue;
        };
        loop {
            let hit = out
                .iter()
                .position(|o| o.x < d.right() && d.x < o.right() && o.y < d.bottom() && d.y < o.bottom());
            match hit {
                Some(i) => {
                    let o = out.swap_remove(i);
                    let l = o.x.min(d.x);
                    let t = o.y.min(d.y);
                    let rr = o.right().max(d.right());
                    let b = o.bottom().max(d.bottom());
                    d = DeviceRect::new(l, t, (rr - l) as u32, (b - t) as u32);
                }
                None => break,
            }
        }
        out.push(d);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_overlapping_and_caps_count() {
        let mut d = Damage::default();
        d.add(Rect::new(0.0, 0.0, 10.0, 10.0));
        d.add(Rect::new(5.0, 5.0, 10.0, 10.0));
        d.add(Rect::new(100.0, 100.0, 5.0, 5.0));
        let set = d.take();
        assert_eq!(set.rects.len(), 2);
        assert!(set.rects.contains(&Rect::new(0.0, 0.0, 15.0, 15.0)));
        assert!(d.is_empty());

        for i in 0..20 {
            d.add(Rect::new(i as f32 * 50.0, 0.0, 10.0, 10.0));
        }
        assert!(d.take().rects.len() <= MAX_RECTS);
    }

    #[test]
    fn device_conversion_covers_and_merges() {
        let bounds = DeviceRect::new(0, 0, 1000, 1000);
        let out = to_device(&[Rect::new(10.2, 10.2, 5.0, 5.0)], 1.25, 0.0, bounds);
        assert_eq!(out, vec![DeviceRect::new(12, 12, 7, 7)]);
        let out = to_device(
            &[Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(9.0, 0.0, 10.0, 10.0)],
            1.0,
            0.0,
            bounds,
        );
        assert_eq!(out, vec![DeviceRect::new(0, 0, 19, 10)]);
    }
}
