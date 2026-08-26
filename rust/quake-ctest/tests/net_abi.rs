//! ABI cross-check: the `quake_types::net` mirrors vs what the engine's own
//! headers (common.h sizebuf, net.h, net_defs.h) say on this platform
//! (Phase 5). Under `-Duse_rust_net` the Rust wire layer shares `net_message`
//! and `qsocket_t` storage with C and its driver functions are installed into
//! the C `net_drivers[]`/`net_landrivers[]` vtables, so mirror drift is
//! silent memory corruption rather than a link error. The net headers are not
//! bindgen-clean roots, so the mirrors are hand-written (ADR-011) and this
//! probe, compiled from the engine's own headers, is the per-platform gate.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::net::{
    self, HostCache, NetDriver, NetLanDriver, PollProcedure, QHostAddr, QSockAddr, QSocket,
    SizeBuf, SysSocket,
};

extern "C" {
    fn ctest_abi_net_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_net_lookup(cstr.as_ptr()) };
    assert_ne!(v, usize::MAX, "key {key:?} missing from the C probe table");
    v
}

macro_rules! check_size {
    ($rust:ty, $ctag:literal) => {
        assert_eq!(
            size_of::<$rust>(),
            c_abi(concat!("sizeof.", $ctag)),
            concat!("sizeof ", $ctag)
        );
    };
}

macro_rules! check_offset {
    ($rust:ty, $field:ident, $ckey:literal) => {
        assert_eq!(offset_of!($rust, $field), c_abi($ckey), $ckey);
    };
}

#[test]
fn net_mirrors_match_engine_headers() {
    check_size!(SizeBuf, "sizebuf_t");
    check_offset!(SizeBuf, allowoverflow, "sizebuf_t.allowoverflow");
    check_offset!(SizeBuf, overflowed, "sizebuf_t.overflowed");
    check_offset!(SizeBuf, data, "sizebuf_t.data");
    check_offset!(SizeBuf, maxsize, "sizebuf_t.maxsize");
    check_offset!(SizeBuf, cursize, "sizebuf_t.cursize");

    check_size!(QSockAddr, "qsockaddr");
    check_offset!(QSockAddr, qsa_family, "qsockaddr.qsa_family");
    check_offset!(QSockAddr, qsa_data, "qsockaddr.qsa_data");

    check_size!(QSocket, "qsocket_t");
    check_offset!(QSocket, next, "qsocket_t.next");
    check_offset!(QSocket, connecttime, "qsocket_t.connecttime");
    check_offset!(QSocket, last_message_time, "qsocket_t.lastMessageTime");
    check_offset!(QSocket, last_send_time, "qsocket_t.lastSendTime");
    check_offset!(QSocket, isvirtual, "qsocket_t.isvirtual");
    check_offset!(QSocket, disconnected, "qsocket_t.disconnected");
    check_offset!(QSocket, can_send, "qsocket_t.canSend");
    check_offset!(QSocket, send_next, "qsocket_t.sendNext");
    check_offset!(QSocket, driver, "qsocket_t.driver");
    check_offset!(QSocket, landriver, "qsocket_t.landriver");
    check_offset!(QSocket, socket, "qsocket_t.socket");
    check_offset!(QSocket, driverdata, "qsocket_t.driverdata");
    check_offset!(QSocket, ack_sequence, "qsocket_t.ackSequence");
    check_offset!(QSocket, send_sequence, "qsocket_t.sendSequence");
    check_offset!(
        QSocket,
        unreliable_send_sequence,
        "qsocket_t.unreliableSendSequence"
    );
    check_offset!(QSocket, send_message_length, "qsocket_t.sendMessageLength");
    check_offset!(QSocket, send_message, "qsocket_t.sendMessage");
    check_offset!(QSocket, receive_sequence, "qsocket_t.receiveSequence");
    check_offset!(
        QSocket,
        unreliable_receive_sequence,
        "qsocket_t.unreliableReceiveSequence"
    );
    check_offset!(
        QSocket,
        receive_message_length,
        "qsocket_t.receiveMessageLength"
    );
    check_offset!(QSocket, receive_message, "qsocket_t.receiveMessage");
    check_offset!(QSocket, addr, "qsocket_t.addr");
    check_offset!(QSocket, trueaddress, "qsocket_t.trueaddress");
    check_offset!(QSocket, maskedaddress, "qsocket_t.maskedaddress");
    check_offset!(
        QSocket,
        proquake_angle_hack,
        "qsocket_t.proquake_angle_hack"
    );
    check_offset!(QSocket, max_datagram, "qsocket_t.max_datagram");
    check_offset!(
        QSocket,
        pending_max_datagram,
        "qsocket_t.pending_max_datagram"
    );

    check_size!(NetLanDriver, "net_landriver_t");
    check_offset!(NetLanDriver, name, "net_landriver_t.name");
    check_offset!(NetLanDriver, initialized, "net_landriver_t.initialized");
    check_offset!(NetLanDriver, control_sock, "net_landriver_t.controlSock");
    check_offset!(NetLanDriver, init, "net_landriver_t.Init");
    check_offset!(NetLanDriver, shutdown, "net_landriver_t.Shutdown");
    check_offset!(NetLanDriver, listen, "net_landriver_t.Listen");
    check_offset!(
        NetLanDriver,
        query_addresses,
        "net_landriver_t.QueryAddresses"
    );
    check_offset!(NetLanDriver, open_socket, "net_landriver_t.Open_Socket");
    check_offset!(NetLanDriver, close_socket, "net_landriver_t.Close_Socket");
    check_offset!(NetLanDriver, connect, "net_landriver_t.Connect");
    check_offset!(
        NetLanDriver,
        check_new_connections,
        "net_landriver_t.CheckNewConnections"
    );
    check_offset!(NetLanDriver, read, "net_landriver_t.Read");
    check_offset!(NetLanDriver, write, "net_landriver_t.Write");
    check_offset!(NetLanDriver, broadcast, "net_landriver_t.Broadcast");
    check_offset!(NetLanDriver, addr_to_string, "net_landriver_t.AddrToString");
    check_offset!(NetLanDriver, string_to_addr, "net_landriver_t.StringToAddr");
    check_offset!(
        NetLanDriver,
        get_socket_addr,
        "net_landriver_t.GetSocketAddr"
    );
    check_offset!(
        NetLanDriver,
        get_name_from_addr,
        "net_landriver_t.GetNameFromAddr"
    );
    check_offset!(
        NetLanDriver,
        get_addr_from_name,
        "net_landriver_t.GetAddrFromName"
    );
    check_offset!(NetLanDriver, addr_compare, "net_landriver_t.AddrCompare");
    check_offset!(
        NetLanDriver,
        get_socket_port,
        "net_landriver_t.GetSocketPort"
    );
    check_offset!(
        NetLanDriver,
        set_socket_port,
        "net_landriver_t.SetSocketPort"
    );
    check_offset!(
        NetLanDriver,
        listening_sock,
        "net_landriver_t.listeningSock"
    );

    check_size!(HostCache, "hostcache_t");
    check_offset!(HostCache, name, "hostcache_t.name");
    check_offset!(HostCache, map, "hostcache_t.map");
    check_offset!(HostCache, gamedir, "hostcache_t.gamedir");
    check_offset!(HostCache, cname, "hostcache_t.cname");
    check_offset!(HostCache, users, "hostcache_t.users");
    check_offset!(HostCache, maxusers, "hostcache_t.maxusers");
    check_offset!(HostCache, driver, "hostcache_t.driver");
    check_offset!(HostCache, ldriver, "hostcache_t.ldriver");
    check_offset!(HostCache, addr, "hostcache_t.addr");

    check_size!(PollProcedure, "PollProcedure");
    check_offset!(PollProcedure, next, "PollProcedure.next");
    check_offset!(PollProcedure, next_time, "PollProcedure.nextTime");
    check_offset!(PollProcedure, procedure, "PollProcedure.procedure");
    check_offset!(PollProcedure, arg, "PollProcedure.arg");

    check_size!(NetDriver, "net_driver_t");
    check_offset!(NetDriver, name, "net_driver_t.name");
    check_offset!(NetDriver, initialized, "net_driver_t.initialized");
    check_offset!(NetDriver, init, "net_driver_t.Init");
    check_offset!(NetDriver, listen, "net_driver_t.Listen");
    check_offset!(NetDriver, query_addresses, "net_driver_t.QueryAddresses");
    check_offset!(NetDriver, search_for_hosts, "net_driver_t.SearchForHosts");
    check_offset!(NetDriver, connect, "net_driver_t.Connect");
    check_offset!(
        NetDriver,
        check_new_connections,
        "net_driver_t.CheckNewConnections"
    );
    check_offset!(NetDriver, qget_any_message, "net_driver_t.QGetAnyMessage");
    check_offset!(NetDriver, qget_message, "net_driver_t.QGetMessage");
    check_offset!(NetDriver, qsend_message, "net_driver_t.QSendMessage");
    check_offset!(
        NetDriver,
        send_unreliable_message,
        "net_driver_t.SendUnreliableMessage"
    );
    check_offset!(NetDriver, can_send_message, "net_driver_t.CanSendMessage");
    check_offset!(
        NetDriver,
        can_send_unreliable_message,
        "net_driver_t.CanSendUnreliableMessage"
    );
    check_offset!(NetDriver, close, "net_driver_t.Close");
    check_offset!(NetDriver, shutdown, "net_driver_t.Shutdown");

    assert_eq!(size_of::<SysSocket>(), c_abi("sizeof.sys_socket_t"));
    assert_eq!(size_of::<QHostAddr>(), c_abi("sizeof.qhostaddr_t"));
}

#[test]
fn net_consts_match_engine_headers() {
    assert_eq!(net::NET_NAMELEN, c_abi("const.NET_NAMELEN"));
    assert_eq!(net::NET_MAXMESSAGE, c_abi("const.NET_MAXMESSAGE"));
    assert_eq!(net::MAX_MSGLEN, c_abi("const.MAX_MSGLEN"));
    assert_eq!(net::MAX_DATAGRAM, c_abi("const.MAX_DATAGRAM"));
    assert_eq!(net::DATAGRAM_MTU, c_abi("const.DATAGRAM_MTU"));
    assert_eq!(net::NET_HEADERSIZE, c_abi("const.NET_HEADERSIZE"));
    assert_eq!(net::NET_DATAGRAMSIZE, c_abi("const.NET_DATAGRAMSIZE"));
    assert_eq!(
        net::NETFLAG_LENGTH_MASK as usize,
        c_abi("const.NETFLAG_LENGTH_MASK")
    );
    assert_eq!(net::NETFLAG_DATA as usize, c_abi("const.NETFLAG_DATA"));
    assert_eq!(net::NETFLAG_ACK as usize, c_abi("const.NETFLAG_ACK"));
    assert_eq!(net::NETFLAG_NAK as usize, c_abi("const.NETFLAG_NAK"));
    assert_eq!(net::NETFLAG_EOM as usize, c_abi("const.NETFLAG_EOM"));
    assert_eq!(
        net::NETFLAG_UNRELIABLE as usize,
        c_abi("const.NETFLAG_UNRELIABLE")
    );
    assert_eq!(net::NETFLAG_CTL as usize, c_abi("const.NETFLAG_CTL"));
    assert_eq!(net::NET_LOOPBACKBUFFERS, c_abi("const.NET_LOOPBACKBUFFERS"));
    assert_eq!(
        net::NET_LOOPBACKHEADERSIZE,
        c_abi("const.NET_LOOPBACKHEADERSIZE")
    );
    assert_eq!(
        net::NET_PROTOCOL_VERSION as usize,
        c_abi("const.NET_PROTOCOL_VERSION")
    );
    for (v, k) in [
        (net::CCREQ_CONNECT, "const.CCREQ_CONNECT"),
        (net::CCREQ_SERVER_INFO, "const.CCREQ_SERVER_INFO"),
        (net::CCREQ_PLAYER_INFO, "const.CCREQ_PLAYER_INFO"),
        (net::CCREQ_RULE_INFO, "const.CCREQ_RULE_INFO"),
        (net::CCREQ_RCON, "const.CCREQ_RCON"),
        (net::CCREP_ACCEPT, "const.CCREP_ACCEPT"),
        (net::CCREP_REJECT, "const.CCREP_REJECT"),
        (net::CCREP_SERVER_INFO, "const.CCREP_SERVER_INFO"),
        (net::CCREP_PLAYER_INFO, "const.CCREP_PLAYER_INFO"),
        (net::CCREP_RULE_INFO, "const.CCREP_RULE_INFO"),
        (net::CCREP_RCON, "const.CCREP_RCON"),
    ] {
        assert_eq!(v as usize, c_abi(k), "{k}");
    }
    assert_eq!(net::HOSTCACHESIZE, c_abi("const.HOSTCACHESIZE"));

    // SA_FAM_OFFSET probes whether the platform headers took the HAVE_SA_LEN
    // (BSD sockaddr) branch; the mirror's cfg ladder must agree
    let expected_fam_offset = offset_of!(QSockAddr, qsa_family);
    assert_eq!(expected_fam_offset, c_abi("const.SA_FAM_OFFSET"));
}
