//! Interactive shape-bound GFX canvas. Only verified DefineShape rectangles
//! are drawn or draggable; sprite/runtime classes without bounds remain numeric-only.
use evolve_core::StaticLayer;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Event, Point, Rectangle, Renderer, Size, Theme, mouse};

pub struct GfxLayoutCanvas<'a> {
    pub layers: &'a [StaticLayer],
    pub selected: Option<usize>,
    pub stage: Size,
}
#[derive(Default)]
pub struct DragState {
    pressed: Option<(Point, usize)>,
    hover: Option<Point>,
}
fn transform(layer: &StaticLayer, point: Point) -> Point {
    let (sin, cos) = layer.rotation.to_radians().sin_cos();
    let x = point.x * layer.scale_x;
    let y = point.y * layer.scale_y;
    Point::new(layer.x + x*cos - y*sin, layer.y + x*sin + y*cos)
}
fn projection(size: Size, bounds: Rectangle) -> (f32, Point) {
    let scale=(bounds.width / size.width.max(1.)).min(bounds.height / size.height.max(1.));
    let offset=Point::new((bounds.width-size.width*scale)/2.,(bounds.height-size.height*scale)/2.);
    (scale,offset)
}
fn points(layer: &StaticLayer,scale:f32,offset:Point)->Option<[Point;4]> {
    let [xmin,xmax,ymin,ymax]=layer.bounds?;
    let corners=[
        Point::new(xmin,ymin),Point::new(xmax,ymin),
        Point::new(xmax,ymax),Point::new(xmin,ymax)
    ];
    Some(corners.map(|p|{
        let pos=transform(layer,p);
        Point::new(offset.x+pos.x*scale,offset.y+pos.y*scale)
    }))
}
fn inside(p:Point,poly:[Point;4])->bool{
    // Convex rotated quad, rejecting points outside without approximating a
    // sprite/bitmap rectangle not specified by authored SWF shape bounds.
    let mut sign=0f32;
    for i in 0..4 {
        let a=poly[i];let b=poly[(i+1)%4];
        let cross=(b.x-a.x)*(p.y-a.y)-(b.y-a.y)*(p.x-a.x);
        if cross.abs()<0.001 {continue;}
        if sign!=0. && sign*cross < 0. {return false;}
        sign=cross;
    }
    true
}
impl canvas::Program<super::Message> for GfxLayoutCanvas<'_> {
    type State=DragState;
    fn update(
        &self,state:&mut Self::State,event:&Event,bounds:Rectangle,cursor:mouse::Cursor
    )->Option<canvas::Action<super::Message>> {
        let (scale,offset)=projection(self.stage,bounds);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))=>{
                let position=cursor.position_in(bounds)?;
                let picked=self.layers.iter().enumerate().rev()
                    .filter(|(_,layer)|layer.editable)
                    .find(|(_,layer)|points(layer,scale,offset).is_some_and(|polygon|inside(position,polygon)))?
                    .0;
                state.pressed=Some((position,picked));
                state.hover=Some(position);
                Some(canvas::Action::publish(super::Message::GfxLayoutSelect(picked)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))=>{
                let (begin,index)=state.pressed.take()?;
                state.hover=None;
                let end=cursor.position_in(bounds)?;
                let layer=self.layers.get(index)?;
                if !layer.editable || scale<=0. {return None;}
                let delta=Point::new((end.x-begin.x)/scale,(end.y-begin.y)/scale);
                if delta.x.abs()>0.01 || delta.y.abs()>0.01 {
                    Some(canvas::Action::publish(super::Message::GfxLayoutDragged(index,delta.x,delta.y)).and_capture())
                } else {
                    Some(canvas::Action::capture())
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.pressed.is_some() => {
                state.hover = cursor.position_in(bounds);
                Some(canvas::Action::capture())
            }
            _=>None
        }
    }
    fn draw(
        &self,state:&Self::State,renderer:&Renderer,_theme:&Theme,bounds:Rectangle,
        _cursor:mouse::Cursor
    )->Vec<canvas::Geometry> {
        let mut frame=canvas::Frame::new(renderer,bounds.size());
        frame.fill_rectangle(Point::ORIGIN,frame.size(),Color::from_rgb8(23,24,30));
        let (scale,offset)=projection(self.stage,bounds);
        if scale<=0. {return vec![frame.into_geometry()];}
        frame.stroke(&Path::rectangle(offset,Size::new(
            self.stage.width*scale,self.stage.height*scale
        )),Stroke::default().with_color(Color::from_rgb8(95,103,115)).with_width(1.));
        for (index,layer) in self.layers.iter().enumerate(){
            if let Some(mut poly)=points(layer,scale,offset) {
                if let Some((start,dragged))=state.pressed
                    && dragged==index
                    && let Some(hover)=state.hover {
                    for point in &mut poly {
                        point.x+=hover.x-start.x;
                        point.y+=hover.y-start.y;
                    }
                }
                let path=Path::new(|p|{
                    p.move_to(poly[0]);
                    for point in poly.into_iter().skip(1){p.line_to(point);}
                    p.close();
                });
                let selected=self.selected==Some(index);
                let color=if !layer.editable{Color::from_rgb8(128,129,141)}
                    else if selected {Color::from_rgb8(255,196,85)}
                    else {Color::from_rgb8(237,49,49)};
                frame.stroke(&path,Stroke::default().with_color(color).with_width(if selected{3.}else{1.5}));
            }
        }
        vec![frame.into_geometry()]
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn rotated_rectangle_hit_test_does_not_use_unbounded_guess(){
        let layer=StaticLayer{
            depth:1,character:3,label:"test".into(),bounds:Some([0.,100.,0.,30.]),
            x:100.,y:50.,scale_x:1.,scale_y:1.,rotation:90.,
            editable:true,reason:"test",
        };
        let box_points=points(&layer,1.,Point::ORIGIN).unwrap();
        assert!(inside(Point::new(85.,100.),box_points));
        assert!(!inside(Point::new(140.,100.),box_points));
    }
}
