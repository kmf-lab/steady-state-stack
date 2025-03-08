use crate::distributed::distributed_stream::StreamItem;
use crate::Rx;
//TODO: these are just draft ideas.

// pub struct StreamIteratorBoxed<'a, T: StreamItem> {
//     item_channel: &'a mut Rx<T>,
//     payload_channel: &'a mut Rx<u8>,
// }
// 
// impl<'a, T: StreamItem> Iterator for StreamIteratorBoxed<'a, T> {
//     type Item = (T, Box<[u8]>);
// 
//     fn next(&mut self) -> Option<Self::Item> {
//         match self.item_channel.next() {
//             Some(item) => {
//                 let length = item.length();
//                 let (parta, partb) = self.payload_channel.slice();
//                 // Check if there's enough data in the combined slices
//                 if parta.len() + partb.len() < length {
//                     return None; // Not enough data available
//                 }
//                 // Create a contiguous buffer
//                 let mut buffer = Vec::with_capacity(length);
//                 let mut remaining = length;
//                 if remaining > 0 {
//                     let a_len = parta.len().min(remaining);
//                     buffer.extend_from_slice(&parta[..a_len]);
//                     remaining -= a_len;
//                 }
//                 if remaining > 0 {
//                     let b_len = partb.len().min(remaining);
//                     buffer.extend_from_slice(&partb[..b_len]);
//                 }
//                 Some((item, buffer.into_boxed_slice()))
//             }
//             None => None, // No more items
//         }
//     }
// }
// 
// pub struct StreamIteratorTwoSlices<'a, T: StreamItem> {
//     item_channel: &'a mut Rx<T>,
//     payload_channel: &'a mut Rx<u8>,
// }
// 
// impl<'a, T: StreamItem> Iterator for StreamIteratorTwoSlices<'a, T> {
//     type Item = (T, &'a [u8], &'a [u8]);
// 
//     fn next(&mut self) -> Option<Self::Item> {
//         match self.item_channel.next() {
//             Some(item) => {
//                 let length = item.length();
//                 let (parta, partb) = self.payload_channel.slice();
//                 // Check if there's enough data in the combined slices
//                 if parta.len() + partb.len() < length {
//                     return None; // Not enough data available
//                 }
//                 // Calculate how much data to take from each slice
//                 let mut remaining = length;
//                 let a_len = parta.len().min(remaining);
//                 remaining -= a_len;
//                 let b_len = partb.len().min(remaining);
//                 Some((item, &parta[..a_len], &partb[..b_len]))
//             }
//             None => None, // No more items
//         }
//     }
// }
