use super::Interface;
use super::MacAddr;
use crate::sys;
use crate::gateway;

use libc;
use std::ffi::{CStr, CString};
use std::mem::{self, MaybeUninit};
use std::os::raw::c_char;
use std::str::from_utf8_unchecked;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[cfg(any(target_os = "macos", target_os = "openbsd", target_os = "freebsd", target_os = "netbsd", target_os = "ios"))]
pub fn interfaces() -> Vec<Interface> {
    let mut interfaces: Vec<Interface> = unix_interfaces();
    let local_ip: IpAddr = match super::get_local_ipaddr(){
        Some(local_ip) => local_ip,
        None => return interfaces,
    };
    for iface in &mut interfaces {
        match local_ip {
            IpAddr::V4(local_ipv4) => {
                if iface.ipv4.contains(&local_ipv4) {
                    match gateway::unix::get_default_gateway(iface.name.clone()) {
                        Ok(gateway) => {
                            iface.gateway = Some(gateway);
                        },
                        Err(_) => {},
                    }
                }
            },
            IpAddr::V6(local_ipv6) => {
                if iface.ipv6.contains(&local_ipv6) {
                    match gateway::unix::get_default_gateway(iface.name.clone()) {
                        Ok(gateway) => {
                            iface.gateway = Some(gateway);
                        },
                        Err(_) => {},
                    }
                }
            },
        }
    }
    interfaces
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn interfaces() -> Vec<Interface> {
    let mut interfaces: Vec<Interface> = unix_interfaces();
    let local_ip: IpAddr = match super::get_local_ipaddr(){
        Some(local_ip) => local_ip,
        None => return interfaces,
    };
    for iface in &mut interfaces {
        match local_ip {
            IpAddr::V4(local_ipv4) => {
                if iface.ipv4.contains(&local_ipv4) {
                    match gateway::linux::get_default_gateway(iface.name.clone()) {
                        Ok(gateway) => {
                            iface.gateway = Some(gateway);
                        },
                        Err(_) => {},
                    }
                }
            },
            IpAddr::V6(local_ipv6) => {
                if iface.ipv6.contains(&local_ipv6) {
                    match gateway::linux::get_default_gateway(iface.name.clone()) {
                        Ok(gateway) => {
                            iface.gateway = Some(gateway);
                        },
                        Err(_) => {},
                    }
                }
            },
        }
    }
    interfaces
}

pub fn unix_interfaces() -> Vec<Interface> {
    let mut ifaces: Vec<Interface> = vec![];
    let mut addrs: MaybeUninit<*mut libc::ifaddrs> = MaybeUninit::uninit();
    if unsafe { libc::getifaddrs(addrs.as_mut_ptr()) } != 0 {
        return ifaces;
    }
    let addrs = unsafe { addrs.assume_init() };
    let mut addr = addrs;
    while !addr.is_null() {
        let addr_ref: &libc::ifaddrs = unsafe {&*addr};
        let c_str = addr_ref.ifa_name as *const c_char;
        let bytes = unsafe { CStr::from_ptr(c_str).to_bytes() };
        let name = unsafe {from_utf8_unchecked(bytes).to_owned() };
        let (mac, ip) = sockaddr_to_network_addr(addr_ref.ifa_addr as *const libc::sockaddr);
        let mut ini_ipv4: Vec<Ipv4Addr> = vec![];
        let mut ini_ipv6: Vec<Ipv6Addr> = vec![];
        if let Some(ip) = ip {
            match ip {
                IpAddr::V4(ipv4) => {
                    ini_ipv4.push(ipv4);
                },
                IpAddr::V6(ipv6) => {
                    ini_ipv6.push(ipv6);
                },
            }
        }
        let interface: Interface = Interface{
            index: 0,
            name: name.clone(),
            description: None,
            mac_addr: mac.clone(),
            ipv4: ini_ipv4,
            ipv6: ini_ipv6,
            gateway: None,
        };
        let mut found: bool = false;
        for iface in &mut ifaces {
            if name == iface.name {
                if let Some(mac) = mac.clone() {
                    iface.mac_addr = Some(mac);
                }
                if let Some(ip) = ip {
                    match ip {
                        IpAddr::V4(ipv4) => {
                            iface.ipv4.push(ipv4);
                        },
                        IpAddr::V6(ipv6) => {
                            iface.ipv6.push(ipv6);
                        },
                    }
                }
                found = true;
            }
        }
        if !found {
            ifaces.push(interface);
        }
        addr = addr_ref.ifa_next;
    }
    unsafe{ libc::freeifaddrs(addrs); } 
    for iface in &mut ifaces {
        let name = CString::new(iface.name.as_bytes()).unwrap();
        unsafe { iface.index = libc::if_nametoindex(name.as_ptr()); }
    }
    ifaces
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn sockaddr_to_network_addr(sa: *const libc::sockaddr) -> (Option<MacAddr>, Option<IpAddr>) {
    use std::net::SocketAddr;

    unsafe {
        if sa.is_null() {
            (None, None)
        } else if (*sa).sa_family as libc::c_int == libc::AF_PACKET {
            let sll: *const libc::sockaddr_ll = mem::transmute(sa);
            let mac = MacAddr(
                (*sll).sll_addr[0],
                (*sll).sll_addr[1],
                (*sll).sll_addr[2],
                (*sll).sll_addr[3],
                (*sll).sll_addr[4],
                (*sll).sll_addr[5],
            );

            (Some(mac), None)
        } else {
            let addr = sys::sockaddr_to_addr(
                mem::transmute(sa),
                mem::size_of::<libc::sockaddr_storage>(),
            );

            match addr {
                Ok(SocketAddr::V4(sa)) => (None, Some(IpAddr::V4(*sa.ip()))),
                Ok(SocketAddr::V6(sa)) => (None, Some(IpAddr::V6(*sa.ip()))),
                Err(_) => (None, None),
            }
        }
    }
}

#[cfg(any(target_os = "openbsd", target_os = "freebsd", target_os = "netbsd", target_os = "macos", target_os = "ios"))]
fn sockaddr_to_network_addr(sa: *const libc::sockaddr) -> (Option<MacAddr>, Option<IpAddr>) {
    use crate::bpf;
    use std::net::SocketAddr;

    unsafe {
        if sa.is_null() {
            (None, None)
        } else if (*sa).sa_family as libc::c_int == bpf::AF_LINK {
            let sdl: *const bpf::sockaddr_dl = mem::transmute(sa);
            let nlen = (*sdl).sdl_nlen as usize;
            let mac = MacAddr(
                (*sdl).sdl_data[nlen] as u8,
                (*sdl).sdl_data[nlen + 1] as u8,
                (*sdl).sdl_data[nlen + 2] as u8,
                (*sdl).sdl_data[nlen + 3] as u8,
                (*sdl).sdl_data[nlen + 4] as u8,
                (*sdl).sdl_data[nlen + 5] as u8,
            );

            (Some(mac), None)
        } else {
            let addr = sys::sockaddr_to_addr(
                mem::transmute(sa),
                mem::size_of::<libc::sockaddr_storage>(),
            );

            match addr {
                Ok(SocketAddr::V4(sa)) => (None, Some(IpAddr::V4(*sa.ip()))),
                Ok(SocketAddr::V6(sa)) => (None, Some(IpAddr::V6(*sa.ip()))),
                Err(_) => (None, None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_unix_interfaces() {
        let interfaces = interfaces();
        for interface in interfaces {
            println!("{:#?}", interface);
        }
    }
}
