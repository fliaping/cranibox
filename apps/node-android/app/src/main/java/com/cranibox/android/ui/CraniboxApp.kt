package com.cranibox.android.ui

import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Widgets
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import com.cranibox.android.R
import com.cranibox.android.ui.devices.DeviceDetailScreen
import com.cranibox.android.ui.devices.DevicesScreen
import com.cranibox.android.ui.devices.QrScanScreen
import com.cranibox.android.ui.devices.SessionScreen
import com.cranibox.android.ui.home.HomeScreen
import com.cranibox.android.ui.phone.PhoneScreen
import com.cranibox.android.ui.phone.PreviewScreen
import java.net.URLEncoder
import java.nio.charset.StandardCharsets

@Composable
fun CraniboxApp(
    pendingLink: String?,
    onPendingLinkConsumed: () -> Unit,
    openApprovals: Boolean,
    onApprovalsOpened: () -> Unit,
) {
    val nav = rememberNavController()
    val snackbar = remember { SnackbarHostState() }
    val entry by nav.currentBackStackEntryAsState()
    val route = entry?.destination?.route.orEmpty()
    val showBar = route in setOf("home", "devices", "phone")

    LaunchedEffect(openApprovals) {
        if (openApprovals) {
            nav.navigate("home") { launchSingleTop = true }
            onApprovalsOpened()
        }
    }
    LaunchedEffect(pendingLink) {
        val link = pendingLink ?: return@LaunchedEffect
        nav.navigate("scan?link=" + URLEncoder.encode(link, StandardCharsets.UTF_8)) {
            launchSingleTop = true
        }
        onPendingLinkConsumed()
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            if (showBar) {
                NavigationBar {
                    NavItem("home", stringResource(R.string.tab_home), Icons.Outlined.Widgets, route, nav)
                    NavItem("devices", stringResource(R.string.tab_devices), Icons.Outlined.Devices, route, nav)
                    NavItem("phone", stringResource(R.string.tab_phone), Icons.Outlined.PhoneAndroid, route, nav)
                }
            }
        },
    ) { inner ->
        NavHost(navController = nav, startDestination = "home", modifier = Modifier.padding(inner)) {
            composable("home") { HomeScreen(snackbar) }
            composable("devices") {
                DevicesScreen(
                    onDevice = { nav.navigate("device/$it") },
                    onScan = { nav.navigate("scan") },
                )
            }
            composable(
                "device/{id}",
                arguments = listOf(navArgument("id") { type = NavType.StringType }),
            ) { back ->
                val id = back.arguments?.getString("id").orEmpty()
                DeviceDetailScreen(
                    nodeId = id,
                    onBack = { nav.popBackStack() },
                    onSession = { kind -> nav.navigate("session/$id/$kind") },
                    snackbar = snackbar,
                )
            }
            composable(
                "scan?link={link}",
                arguments = listOf(navArgument("link") { type = NavType.StringType; defaultValue = "" }),
            ) { back ->
                QrScanScreen(
                    initialLink = back.arguments?.getString("link").orEmpty(),
                    onBack = { nav.popBackStack() },
                    snackbar = snackbar,
                )
            }
            composable("scan") {
                QrScanScreen(initialLink = "", onBack = { nav.popBackStack() }, snackbar = snackbar)
            }
            composable(
                "session/{id}/{kind}",
                arguments = listOf(
                    navArgument("id") { type = NavType.StringType },
                    navArgument("kind") { type = NavType.StringType },
                ),
            ) { back ->
                SessionScreen(
                    nodeId = back.arguments?.getString("id").orEmpty(),
                    kind = back.arguments?.getString("kind").orEmpty(),
                    onClose = { nav.popBackStack() },
                )
            }
            composable("phone") { PhoneScreen(onPreview = { nav.navigate("preview") }, snackbar = snackbar) }
            composable("preview") { PreviewScreen(onBack = { nav.popBackStack() }) }
        }
    }
}

@Composable
private fun RowScope.NavItem(
    route: String,
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    current: String,
    nav: androidx.navigation.NavHostController,
) {
    NavigationBarItem(
        selected = current == route,
        onClick = {
            nav.navigate(route) {
                popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                launchSingleTop = true
                restoreState = true
            }
        },
        icon = { Icon(icon, contentDescription = label) },
        label = { Text(label) },
    )
}
